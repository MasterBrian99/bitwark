//! The async UCI session: the command loop plus the stdout writer task.
//!
//! # The async shape (why it looks like this)
//!
//! ```text
//!  GUI ──stdin──▶ ┌─────────────────────────────────────────────┐
//!                 │ UciSession::run()                           │
//!                 │  read line → parse → dispatch → out.send()  │
//!                 └──────────────┬──────────────────────────────┘
//!                                │ mpsc channel (String lines)
//!                                ▼
//!                 ┌─────────────────────────────────────────────┐
//!                 │ writer_task() — the ONLY owner of stdout    │
//!                 │  write line + '\n', flush                   │
//!                 └──────────────stdout──▶ GUI ────────────────┘
//! ```
//!
//! Two rules drive this design:
//!
//! 1. **Exactly one task ever writes stdout.** UCI is line-oriented; a
//!    half-written or interleaved line would corrupt the protocol. Funneling
//!    every reply through one channel with one writer makes interleaving
//!    impossible by construction.
//! 2. **The command loop must never block.** UCI requires `isready` to be
//!    answered even while a search runs (UCI spec §2.3). The search lives on its own OS thread (CPU-bound work does not belong
//!    on an async runtime) and feed `info`/`bestmove` lines into this same
//!    channel — the session stays free to answer pings meanwhile.
//!
//! The session is *stateful across a game* (current position, options) but
//! *stateless between commands*: each line is handled on its own, and any
//! line we don't understand is silently ignored (UCI spec §1).

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::board::{Color, Move, Position, parse_fen};
use crate::search::{MATE, MAX_PLY, SearchLimits, tt::TranspositionTable};
use crate::uci::options::EngineOptions;
use crate::uci::parse::{self, UciCommand};

/// Reported by `id name` (UCI spec §3.1). Version comes from Cargo.toml.
pub const ENGINE_NAME: &str = concat!("Bitwark ", env!("CARGO_PKG_VERSION"));

/// Reported by `id author`.
pub const ENGINE_AUTHOR: &str = "Brian";

/// Handle to the running search thread.
struct SearchHandle {
    thread: std::thread::JoinHandle<()>,
    stop: Arc<AtomicBool>,
}

/// Live protocol state for one engine process.
pub struct UciSession {
    /// Sending half of the stdout channel. Cloned
    /// into the search bridge so the search thread can emit `info` lines
    /// through the exact same single-writer path.
    out: mpsc::Sender<String>,
    /// Engine tunables, mutated by `setoption`.
    options: EngineOptions,
    /// `debug on|off` (UCI spec §2.2). Accepted for conformance; perft divide and evaluation breakdowns are
    /// sent through the same channel.
    #[allow(dead_code)]
    debug: bool,
    /// Current board position, updated by `position` commands.
    position: Position,
    /// Shared transposition table (SMP-ready, lock-free).
    tt: std::sync::Arc<TranspositionTable>,
    /// Running search, if any.
    search: Option<SearchHandle>,
}

impl UciSession {
    pub fn new(out: mpsc::Sender<String>) -> Self {
        let tt_mib = EngineOptions::default().hash_mib;
        Self {
            out,
            options: EngineOptions::default(),
            debug: false,
            position: Position::startpos(),
            tt: std::sync::Arc::new(TranspositionTable::new(tt_mib)),
            search: None,
        }
    }

    /// True if a search is currently running.
    fn search_running(&self) -> bool {
        if let Some(h) = &self.search {
            !h.thread.is_finished()
        } else {
            false
        }
    }

    /// Reap a finished search handle (join the thread).  Call before spawning
    /// a new search or on quit.
    fn reap_search_if_finished(&mut self) {
        if let Some(h) = &self.search
            && h.thread.is_finished()
        {
            let h = self.search.take().unwrap();
            let _ = h.thread.join();
        }
    }

    /// The command loop: read stdin forever, return on `quit` or EOF.
    ///
    /// EOF (the GUI closed the pipe or died) is treated exactly like
    /// `quit` — exiting cleanly beats hanging on a dead parent.
    pub async fn run(&mut self) -> std::io::Result<()> {
        let stdin = tokio::io::stdin();
        let mut lines = BufReader::new(stdin).lines();
        loop {
            let line = match lines.next_line().await? {
                Some(line) => line,
                None => return Ok(()),
            };
            match parse::parse_line(&line) {
                UciCommand::Quit => {
                    // Signal any running search to stop before exiting.
                    if let Some(h) = &self.search {
                        h.stop.store(true, Ordering::Relaxed);
                    }
                    return Ok(());
                }
                UciCommand::Uci => self.handle_uci().await,
                UciCommand::IsReady => self.send("readyok").await,
                UciCommand::Debug(on) => self.debug = on,
                UciCommand::SetOption { name, value } => {
                    let recognized = self.options.set(&name, value.as_deref());
                    if recognized && name == "Hash" {
                        if self.search_running() {
                            self.send("info string error: cannot resize hash while searching")
                                .await;
                        } else {
                            let mib = self.options.hash_mib;
                            // Need &mut to resize; clone Arc if shared.
                            // SAFETY: we checked no search is running, so we hold the only Arc.
                            if let Some(tt_mut) = std::sync::Arc::get_mut(&mut self.tt) {
                                tt_mut.resize(mib);
                            } else {
                                // Fallback: allocate a fresh table (old one drops when search releases)
                                self.tt = std::sync::Arc::new(TranspositionTable::new(mib));
                            }
                        }
                    } else if recognized && name == "Clear Hash" {
                        self.tt.clear();
                    }
                }
                // `position` — update the current board, applying the `moves` tail.
                UciCommand::Position { fen, moves } => {
                    let mut new_pos = match fen {
                        None => Position::startpos(),
                        Some(s) => match parse_fen(&s) {
                            Ok(p) => p,
                            Err(e) => {
                                self.send(&format!("info string error: {e}")).await;
                                continue;
                            }
                        },
                    };
                    let mut ok = true;
                    for mv_str in moves {
                        let mv = match Move::parse_uci(&mv_str) {
                            Some(m) => m,
                            None => {
                                self.send(&format!("info string error: illegal move {mv_str}"))
                                    .await;
                                ok = false;
                                break;
                            }
                        };
                        // Verify legality via generate_legal (pseudo + king safety).
                        let mut legal = Vec::new();
                        crate::board::generate_legal(&mut new_pos, &mut legal);
                        if !legal.contains(&mv) {
                            self.send(&format!("info string error: illegal move {mv_str}"))
                                .await;
                            ok = false;
                            break;
                        }
                        new_pos.make_move(mv);
                    }
                    if ok {
                        self.position = new_pos;
                    }
                }
                UciCommand::D => {
                    for line in self.position.display_lines() {
                        self.send(&line).await;
                    }
                }
                UciCommand::Go(params) => {
                    if let Some(depth) = params.perft {
                        // Stockfish-like perft: per-move counts + total.
                        let divide =
                            crate::board::perft::perft_divide(&mut self.position.clone(), depth);
                        let mut total = 0u64;
                        for (mv, nodes) in divide {
                            self.send(&format!("{mv}: {nodes}")).await;
                            total += nodes;
                        }
                        self.send(&format!("Nodes searched: {total}")).await;
                    } else {
                        self.handle_go(params).await;
                    }
                }
                UciCommand::Stop => {
                    if let Some(h) = &self.search {
                        h.stop.store(true, Ordering::Relaxed);
                    }
                }
                UciCommand::PonderHit => {
                    // Pondering not yet implemented (Phase 6) — treat like stop
                    // for correctness: if pondering, stop the ponder search and
                    // the GUI should send a real `go` afterwards.  For now just
                    // signal stop if a search is running.
                    if let Some(h) = &self.search {
                        h.stop.store(true, Ordering::Relaxed);
                    }
                }
                UciCommand::UciNewGame => {
                    self.tt.clear();
                    self.reap_search_if_finished();
                }
                UciCommand::Register | UciCommand::Unknown => {}
            }
        }
    }

    /// Handle a non-perft `go` — spawn the sync search thread.
    async fn handle_go(&mut self, params: crate::uci::parse::GoParams) {
        // If a search is already running, reject this one (GUIs shouldn't do it).
        if self.search_running() {
            self.send("info string error: search already running").await;
            return;
        }
        // Reap any finished handle before spawning.
        self.reap_search_if_finished();

        // Warn about unsupported limits (Phase 6 will handle them).
        if !params.searchmoves.is_empty() {
            self.send("info string searchmoves not yet supported (Phase 6)")
                .await;
        }
        if params.mate.is_some() {
            self.send("info string mate limit not yet supported (Phase 6)")
                .await;
        }
        if params.nodes.is_some() {
            self.send("info string nodes limit not yet supported (Phase 6)")
                .await;
        }
        if params.ponder {
            self.send("info string ponder not yet supported (Phase 6)")
                .await;
        }

        // Build limits.
        let depth = params.depth;
        let mut movetime_ms = params.movetime;
        let infinite = params.infinite || params.ponder;

        // Minimal clock fallback for time controls (Phase 6 is full management).
        if depth.is_none() && movetime_ms.is_none() && !infinite {
            let time_opt = match self.position.side_to_move() {
                Color::White => params.wtime,
                Color::Black => params.btime,
            };
            let inc_opt = match self.position.side_to_move() {
                Color::White => params.winc,
                Color::Black => params.binc,
            };
            if let Some(time) = time_opt {
                let inc = inc_opt.unwrap_or(0);
                let base = if let Some(mtg) = params.movestogo {
                    if mtg > 0 {
                        time / mtg as u64
                    } else {
                        time / 20
                    }
                } else {
                    time / 20
                };
                let mt = base + inc * 3 / 4;
                let overhead = 10u64;
                let clamped = mt.clamp(10, time.saturating_sub(overhead).max(10));
                movetime_ms = Some(clamped);
            }
        }

        let limits = SearchLimits {
            depth,
            movetime_ms,
            infinite,
        };

        // Bump TT generation for this search.
        self.tt.new_search();

        // Spawn the search thread.
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let pos_clone = self.position.clone();
        let out_clone = self.out.clone();
        let tt_clone = std::sync::Arc::clone(&self.tt);

        let thread = std::thread::spawn(move || {
            let mut pos = pos_clone;
            let limits = limits;
            let stop = stop_clone;
            let out = out_clone;
            let tt = tt_clone;

            let result = crate::search::search(&mut pos, limits, &stop, &tt, &mut |event| {
                let score_str = if event.score.abs() >= MATE - MAX_PLY as i32 {
                    let mate_in = if event.score > 0 {
                        (MATE - event.score + 1) / 2
                    } else {
                        -((MATE + event.score + 1) / 2)
                    };
                    format!("mate {mate_in}")
                } else {
                    format!("cp {}", event.score)
                };
                let pv_str = event
                    .pv
                    .iter()
                    .map(|m| m.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                let line = if pv_str.is_empty() {
                    format!(
                        "info depth {} seldepth {} score {} nodes {} nps {} time {}",
                        event.depth,
                        event.seldepth,
                        score_str,
                        event.nodes,
                        event.nps,
                        event.time_ms
                    )
                } else {
                    format!(
                        "info depth {} seldepth {} score {} nodes {} nps {} time {} pv {}",
                        event.depth,
                        event.seldepth,
                        score_str,
                        event.nodes,
                        event.nps,
                        event.time_ms,
                        pv_str
                    )
                };
                let _ = out.blocking_send(line);
            });

            let best_line = if let Some(mv) = result.best_move {
                if result.pv.len() >= 2 {
                    format!("bestmove {} ponder {}", mv, result.pv[1])
                } else {
                    format!("bestmove {}", mv)
                }
            } else {
                "bestmove (none)".to_string()
            };
            let _ = out.blocking_send(best_line);
        });

        self.search = Some(SearchHandle { thread, stop });
    }

    /// Push one line toward stdout via the writer task.
    async fn send(&self, line: &str) {
        // A send failure means the writer is gone (we're shutting down);
        // nothing useful to do but drop the line.
        let _ = self.out.send(line.to_string()).await;
    }

    /// Reply to `uci`: identity, option list, `uciok` (UCI spec §2.1).
    async fn handle_uci(&self) {
        self.send(&format!("id name {ENGINE_NAME}")).await;
        self.send(&format!("id author {ENGINE_AUTHOR}")).await;
        for line in self.options.option_lines() {
            self.send(&line).await;
        }
        self.send("uciok").await;
    }
}

/// Single owner of stdout. Receives complete lines and flushes them.
///
/// We flush per line rather than batching because UCI GUIs parse stdout
/// line by line and expect low latency on `readyok`/`bestmove`.
pub async fn writer_task(mut rx: mpsc::Receiver<String>) {
    let mut stdout = tokio::io::stdout();
    while let Some(line) = rx.recv().await {
        let mut buf = String::with_capacity(line.len() + 1);
        buf.push_str(&line);
        buf.push('\n');
        // Ignore I/O errors: if stdout is broken, the parent/GUI is gone
        // and we might as well stop.
        let _ = stdout.write_all(buf.as_bytes()).await;
        let _ = stdout.flush().await;
    }
}

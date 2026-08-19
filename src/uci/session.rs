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

#![allow(clippy::collapsible_if)]

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::board::{Color, Move, Position, parse_fen};
use crate::search::{MATE, MAX_PLY, SearchLimits, time::TimeControl, tt::TranspositionTable};
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
    tc: Arc<TimeControl>,
    done: Arc<AtomicBool>,
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
    tt: Arc<TranspositionTable>,
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
            tt: Arc::new(TranspositionTable::new(tt_mib)),
            search: None,
        }
    }

    /// True if a search is currently running.
    fn search_running(&self) -> bool {
        if let Some(h) = &self.search {
            // `done` is set by the thread immediately after sending bestmove.
            // Using `done` closes the race where `is_finished()` is still false
            // but bestmove is already out and the GUI has sent the next `go`.
            if h.done.load(Ordering::Relaxed) {
                return false;
            }
            !h.thread.is_finished()
        } else {
            false
        }
    }

    /// Reap a finished search handle (join the thread).  Call before spawning
    /// a new search or on quit.
    fn reap_search_if_finished(&mut self) {
        let is_done = if let Some(h) = &self.search {
            h.done.load(Ordering::Relaxed) || h.thread.is_finished()
        } else {
            false
        };
        if is_done {
            if let Some(h) = self.search.take() {
                let _ = h.thread.join();
            }
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
                    // UCI spec §7: setoption only processed while idle.
                    if self.search_running() {
                        continue;
                    }
                    let recognized = self.options.set(&name, value.as_deref());
                    if recognized && name == "Hash" {
                        let mib = self.options.hash_mib;
                        if let Some(tt_mut) = Arc::get_mut(&mut self.tt) {
                            tt_mut.resize(mib);
                        } else {
                            self.tt = Arc::new(TranspositionTable::new(mib));
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
                UciCommand::Eval => {
                    let bd = crate::eval::breakdown(&self.position);
                    let phase = bd.phase;
                    self.send("Eval breakdown (white perspective):").await;
                    self.send(&format!(" Phase: {phase}/24")).await;
                    self.send("  Term     |    MG    EG  |  Total").await;
                    self.send("-----------+-------------+--------").await;
                    for i in 0..crate::eval::TERM_COUNT {
                        let mg = bd.term_mg[i];
                        let eg = bd.term_eg[i];
                        let total = (mg * phase + eg * (24 - phase)) / 24;
                        let name = crate::eval::TERM_NAMES[i];
                        self.send(&format!(" {name:9} | {mg:5} {eg:5} | {total:5}"))
                            .await;
                    }
                    let total_mg = bd.mg();
                    let total_eg = bd.eg();
                    let total = bd.white_score();
                    self.send("-----------+-------------+--------").await;
                    self.send(&format!(
                        " Total    | {total_mg:5} {total_eg:5} | {total:5}"
                    ))
                    .await;
                    let total_cp = total as f32 / 100.0;
                    self.send(&format!("Total evaluation: {total_cp:.2} (white side)"))
                        .await;
                    let stm = crate::eval::evaluate(&self.position);
                    let stm_cp = stm as f32 / 100.0;
                    self.send(&format!("Total evaluation (side to move): {stm_cp:.2}"))
                        .await;
                }
                UciCommand::Flip => {
                    self.position.flip_side_to_move();
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
                    if let Some(h) = &self.search {
                        h.tc.ponderhit();
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

        // Ponder warning stays until 6d fully implements the workflow;
        // searchmoves/mate/nodes are now handled (6a/6b).
        if params.ponder {
            self.send("info string ponder not yet supported (Phase 6)")
                .await;
        }

        // Build TimeControl.
        let tc = if let Some(mt) = params.movetime {
            Arc::new(TimeControl::for_movetime(mt))
        } else {
            let time_opt = match self.position.side_to_move() {
                Color::White => params.wtime,
                Color::Black => params.btime,
            };
            let inc_opt = match self.position.side_to_move() {
                Color::White => params.winc,
                Color::Black => params.binc,
            };
            if let Some(rem) = time_opt {
                let inc = inc_opt.unwrap_or(0);
                Arc::new(TimeControl::for_clock(
                    rem,
                    inc,
                    params.movestogo,
                    self.options.move_overhead_ms,
                ))
            } else {
                Arc::new(TimeControl::none())
            }
        };
        if params.ponder {
            tc.start_pondering();
        }

        // Build SearchLimits.
        let searchmoves: Vec<Move> = params
            .searchmoves
            .iter()
            .filter_map(|s| Move::parse_uci(s))
            .collect();
        let limits = SearchLimits {
            depth: params.depth,
            nodes: params.nodes,
            mate: params.mate,
            infinite: params.infinite,
            ponder: params.ponder,
            multipv: self.options.multipv,
            searchmoves,
        };

        // Bump TT generation for this search.
        self.tt.new_search();

        // Spawn the search thread.
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let done_clone = Arc::clone(&done);
        let tc_clone = Arc::clone(&tc);
        let pos_clone = self.position.clone();
        let out_clone = self.out.clone();
        let tt_clone = Arc::clone(&self.tt);

        let thread = std::thread::spawn(move || {
            let mut pos = pos_clone;
            let limits = limits;
            let stop = stop_clone;
            let done = done_clone;
            let tc = tc_clone;
            let out = out_clone;
            let tt = tt_clone;

            let result = crate::search::search(&mut pos, limits, &stop, &tc, &tt, &mut |event| {
                let line = match event {
                    crate::search::SearchEvent::Iteration {
                        depth,
                        seldepth,
                        multipv,
                        score,
                        bound,
                        nodes,
                        time_ms,
                        nps,
                        hashfull,
                        pv,
                    } => {
                        let score_base = if score.abs() >= MATE - MAX_PLY as i32 {
                            let mate_in = if score > 0 {
                                (MATE - score + 1) / 2
                            } else {
                                -((MATE + score + 1) / 2)
                            };
                            format!("mate {mate_in}")
                        } else {
                            format!("cp {score}")
                        };
                        let bound_suffix = match bound {
                            Some(crate::search::tt::Bound::Lower) => " lowerbound",
                            Some(crate::search::tt::Bound::Upper) => " upperbound",
                            _ => "",
                        };
                        let pv_str = pv
                            .iter()
                            .map(|m| m.to_string())
                            .collect::<Vec<_>>()
                            .join(" ");
                        if pv_str.is_empty() {
                            format!(
                                "info depth {depth} seldepth {seldepth} multipv {multipv} score {score_base}{bound_suffix} nodes {nodes} nps {nps} hashfull {hashfull} time {time_ms}"
                            )
                        } else {
                            format!(
                                "info depth {depth} seldepth {seldepth} multipv {multipv} score {score_base}{bound_suffix} nodes {nodes} nps {nps} hashfull {hashfull} time {time_ms} pv {pv_str}"
                            )
                        }
                    }
                    crate::search::SearchEvent::CurrMove {
                        depth,
                        currmove,
                        number,
                    } => {
                        format!("info depth {depth} currmove {currmove} currmovenumber {number}")
                    }
                };
                let _ = out.blocking_send(line);
            });

            // Ponder park: if `go ponder` was requested, don't emit bestmove
            // until `ponderhit` (or `stop`) arrives. While pondering we poll.
            if tc.is_pondering() {
                while tc.is_pondering() && !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }

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
            done.store(true, Ordering::Relaxed);
        });

        self.search = Some(SearchHandle {
            thread,
            stop,
            tc,
            done,
        });
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

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

use crate::board::{Position, parse_fen};
use crate::uci::options::EngineOptions;
use crate::uci::parse::{self, UciCommand};

/// Reported by `id name` (UCI spec §3.1). Version comes from Cargo.toml.
pub const ENGINE_NAME: &str = concat!("Bitwark ", env!("CARGO_PKG_VERSION"));

/// Reported by `id author`.
pub const ENGINE_AUTHOR: &str = "Brian";

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
    // Phase 3: a handle to the search thread + its control channel.
}

impl UciSession {
    pub fn new(out: mpsc::Sender<String>) -> Self {
        Self {
            out,
            options: EngineOptions::default(),
            debug: false,
            position: Position::startpos(),
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
                UciCommand::Quit => return Ok(()),
                UciCommand::Uci => self.handle_uci().await,
                UciCommand::IsReady => self.send("readyok").await,
                UciCommand::Debug(on) => self.debug = on,
                UciCommand::SetOption { name, value } => {
                    self.options.set(&name, value.as_deref());
                }
                // `position` — update the current board (Phase 1). Moves tail is
                // accepted but deferred to Phase 2; we hint via `info string`
                // so manual testing is not silently confusing.
                UciCommand::Position { fen, moves } => {
                    let new_pos = match fen {
                        None => Position::startpos(),
                        Some(s) => match parse_fen(&s) {
                            Ok(p) => p,
                            Err(e) => {
                                self.send(&format!("info string error: {e}")).await;
                                continue;
                            }
                        },
                    };
                    if !moves.is_empty() {
                        self.send("info string position moves ignored (Phase 2)")
                            .await;
                    }
                    self.position = new_pos;
                }
                UciCommand::D => {
                    for line in self.position.display_lines() {
                        self.send(&line).await;
                    }
                }
                // Everything below is inert until its phase lands .
                // Replying nothing is always legal; `go` becomes real in Phase 3.
                UciCommand::UciNewGame
                | UciCommand::Go(_)
                | UciCommand::Stop
                | UciCommand::PonderHit
                | UciCommand::Register
                | UciCommand::Unknown => {}
            }
        }
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

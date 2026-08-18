//! Bitwark — a classical chess engine speaking UCI.
//!
//! Process architecture (the async skeleton everything else hangs on):
//!
//! ```text
//!                 stdin ──▶ UciSession ──▶ mpsc channel ──▶ WriterTask ──▶ stdout
//!                            (parses & dispatches)         (single owner of stdout)
//! ```
//!
//! Why this shape?
//!
//! * UCI is *line-oriented*: the GUI sends one command per line and expects
//!   one-line replies. A dedicated writer task that owns stdout guarantees
//!   lines never interleave, no matter how many tasks/threads produce them.
//! * Search runs on its own synchronous *OS thread* (not a tokio task —
//!   search is pure CPU work and must not occupy async workers). That thread
//!   will feed `info`/`bestmove` lines into the same channel, so it can never
//!   race the session's protocol replies.
//! * The session loop stays free to answer `isready` with `readyok` even
//!   while a search is running (a hard UCI requirement).
//!
//! The runtime uses 2 worker threads: the session task awaits stdin, the
//! writer task awaits the channel — both are idle 99.99% of the time.
//!
//! CLI vs UCI dispatch: `clap` parses OS-shell arguments *before* the tokio
//! runtime is built. No args → UCI mode (the GUI path). `bench` and other
//! subcommands are one-shot and exit without ever starting the UCI loop,
//! mirroring `./stockfish bench` (UCI spec §5.1).

mod bench;
mod board;
mod cli;
mod eval;
mod search;
mod uci;

use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Perft { depth, fen }) => {
            let mut pos = if fen == "startpos" {
                crate::board::Position::startpos()
            } else {
                match crate::board::parse_fen(&fen) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("invalid fen: {e}");
                        std::process::exit(1);
                    }
                }
            };
            let start = std::time::Instant::now();
            // For CLI perft we print divide like Stockfish
            let divide = crate::board::perft::perft_divide(&mut pos.clone(), depth);
            let total = if divide.is_empty() {
                let n = crate::board::perft(&mut pos, depth);
                println!("\nNodes searched: {n}");
                n
            } else {
                let mut tot = 0u64;
                for (mv, nodes) in divide {
                    println!("{mv}: {nodes}");
                    tot += nodes;
                }
                println!("\nNodes searched: {tot}");
                tot
            };
            let elapsed = start.elapsed();
            eprintln!(
                "Time: {} ms ({} nps)",
                elapsed.as_millis(),
                if elapsed.as_millis() > 0 {
                    total * 1000 / elapsed.as_millis() as u64
                } else {
                    total
                }
            );
            std::process::exit(0);
        }
        Some(Commands::Bench {
            tt_size,
            threads,
            limit,
            fen_file,
            limit_type,
        }) => {
            bench::run_bench(tt_size, threads, limit, &fen_file, &limit_type);
            std::process::exit(0);
        }
        None => {
            // UCI mode — the normal GUI path. Build a small tokio runtime
            // (2 workers: session + writer) and block on the session.
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("failed to build tokio runtime");
            let code = rt.block_on(run_uci());
            std::process::exit(code);
        }
    }
}

async fn run_uci() -> i32 {
    // Bounded channel: if the GUI stops reading our output, we block instead
    // of buffering without limit (back-pressure beats OOM).
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(1024);

    let writer = tokio::spawn(uci::session::writer_task(rx));
    let mut session = uci::session::UciSession::new(tx);

    let result = session.run().await;

    // Dropping the session closes the sender half, which lets the writer
    // task drain its remaining lines and exit cleanly.
    drop(session);
    let _ = writer.await;

    // A stdin read error is the only failure mode here; treat it like `quit`.
    if result.is_ok() { 0 } else { 1 }
}

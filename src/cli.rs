//! CLI argument parsing (OS shell → engine).
//!
//! The engine has two entry points, just like Stockfish:
//!
//! * **UCI mode** (default, no CLI args): the GUI launches `bitwark` with no
//!   arguments and speaks UCI on stdin/stdout. This is the normal path.
//! * **One-shot CLI mode**: `bitwark bench [args]` runs a benchmark suite
//!   and exits, mirroring `./stockfish bench` (UCI spec §5.1). Future one-shot
//!   commands (e.g. `perft`) will live here too.
//!
//! We use `clap` with `derive` so the help/version text stays in sync with
//! `Cargo.toml` — `clap` reads `version`/`about` from there automatically.
//! The parser is synchronous and runs *before* the tokio runtime is built,
//! so one-shot commands don't pay the cost of starting an async runtime.

use clap::{Parser, Subcommand};

/// Bitwark — a classical chess engine with a UCI interface.
///
/// When launched without arguments the engine enters UCI mode and speaks
/// the protocol on stdin/stdout (the way every GUI launches it). One-shot
/// commands like `bench` are subcommands.
#[derive(Parser, Debug)]
#[command(name = "bitwark", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run perft (move generation correctness test) on a position.
    ///
    /// Example: `bitwark perft 5` (startpos depth 5) or
    /// `bitwark perft 4 --fen "r3k2r/... w KQkq - 0 1"`
    Perft {
        /// Depth to search (1..255)
        depth: u8,

        /// FEN string (use quotes) or "startpos" (default)
        #[arg(long, default_value = "startpos")]
        fen: String,
    },

    /// Run a fixed benchmark suite and report nodes/second.
    ///
    /// Mirrors Stockfish's `bench` (UCI spec §5.1). With no arguments the
    /// defaults match Stockfish's: `bench 16 1 13 default depth`.
    /// Delegates to `src/bench.rs`, which runs the fixed suite.
    Bench {
        /// Transposition-table size in MiB.
        #[arg(default_value_t = 16)]
        tt_size: usize,

        /// Number of search threads (1 = deterministic; >1 = Lazy SMP, nondeterministic node counts).
        #[arg(default_value_t = 1)]
        threads: usize,

        /// Numeric limit paired with `limit_type` (e.g. depth 13).
        #[arg(default_value_t = 13)]
        limit: usize,

        /// FEN source: "default" (built-in suite), "current" (current
        /// position), or a path to a file with one FEN per line.
        #[arg(default_value = "default")]
        fen_file: String,

        /// What `limit` means: depth, nodes, movetime, perft, eval, ...
        #[arg(default_value = "depth")]
        limit_type: String,
    },
}

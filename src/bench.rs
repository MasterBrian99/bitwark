//! Benchmark harness — deterministic node signature.
//!
//! Mirrors Stockfish's `bench` (UCI spec §5.1) in spirit: a fixed suite of
//! positions searched at a fixed depth, with the total node count reported.
//! The suite is ~12 curated FENs covering quiet, tactical, and endgame
//! character. Because the search is depth-limited, single-threaded, and
//! shares a single TT in a fixed order, `Nodes searched` is bit-for-bit
//! identical across runs on the same binary — the property the gate tests.
//!
//! The bench is a CLI subcommand (`bitwark bench`) and also callable as
//! `run_bench` for the `bench` binary path. Time and NPS vary with the
//! machine, but the signature does not.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use crate::board::fen::parse_fen;
use crate::search::{SearchLimits, time::TimeControl, tt::TranspositionTable};

/// Default FEN suite (~12 positions). Keep it stable — the signature depends on it.
pub const DEFAULT_FENS: &[&str] = &[
    // startpos
    "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    // Kiwipete
    "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    // CPW position 3
    "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
    // CPW position 4
    "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
    // CPW position 5
    "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
    // middlegame
    "r1bq1rk1/pp2ppbp/2np1np1/2p5/2P1P3/1PN1B3/PB1Q1PPP/R3K2R w KQ - 0 1",
    // tactical
    "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R b KQkq - 0 1",
    // endgame - pawn
    "4k3/4p3/8/2p5/2P5/8/4P3/4K3 w - - 0 1",
    // queen vs rook
    "r4rk1/1pp1qppp/p1pb4/nP6/4N3/2p5/1PPnQPPP/R1B1K1NR w KQ - 0 1",
    // open position
    "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3",
    // promo
    "8/P7/8/8/8/8/8/4K2k w - - 0 1",
    // random
    "r1bqk2r/ppp2ppp/2n5/2b1p3/2B1P3/3P1N2/PPP2PPP/RNBQK2R w KQkq - 0 1",
];

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BenchStats {
    pub nodes: u64,
    pub time_ms: u64,
    pub nps: u64,
}

/// Run the bench suite.
///
/// * `tt_mib` — TT size in MiB.
/// * `threads` — number of search threads (Lazy SMP). `1` is deterministic;
///   `>1` is honored but `Nodes searched` becomes nondeterministic due to TT
///   races (the gate only asserts determinism at threads=1).
/// * `limit` — numeric limit (depth for `limit_type == "depth"`).
/// * `fen_file` — `"default"`, `"current"`, or a path to a file with one FEN per line.
/// * `limit_type` — `"depth"` (deterministic) or others (accepted for convenience).
pub fn run_bench(
    tt_mib: usize,
    threads: usize,
    limit: usize,
    fen_file: &str,
    limit_type: &str,
) -> BenchStats {
    // Resolve FENs.
    let fens: Vec<String> = match fen_file {
        "default" => DEFAULT_FENS.iter().map(|s| s.to_string()).collect(),
        "current" => vec!["rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1".to_string()],
        other => {
            // Try as file path, else treat as single FEN inline.
            let path = std::path::Path::new(other);
            if path.exists() {
                let content = std::fs::read_to_string(path).unwrap_or_default();
                let mut out = Vec::new();
                for line in content.lines() {
                    let t = line.trim();
                    if t.is_empty() || t.starts_with('#') {
                        continue;
                    }
                    out.push(t.to_string());
                }
                if out.is_empty() {
                    DEFAULT_FENS.iter().map(|s| s.to_string()).collect()
                } else {
                    out
                }
            } else {
                // Single FEN inline (check if it parses)
                match parse_fen(other) {
                    Ok(_) => vec![other.to_string()],
                    Err(_) => {
                        eprintln!("bench: invalid fen_file '{other}', using default suite");
                        DEFAULT_FENS.iter().map(|s| s.to_string()).collect()
                    }
                }
            }
        }
    };

    // Only depth is deterministic; other types are accepted but noted.
    let is_depth = limit_type == "depth";
    if !is_depth && limit_type != "movetime" && limit_type != "nodes" {
        eprintln!(
            "bench: unsupported limit_type '{limit_type}' — supported: depth, movetime, nodes"
        );
        std::process::exit(1);
    }
    if !is_depth {
        eprintln!("bench: limit_type '{limit_type}' is not deterministic; signature may vary");
    }

    let tt = Arc::new(TranspositionTable::new(tt_mib as u32));
    let depth_limit = if is_depth { Some(limit as u8) } else { None };
    let movetime_limit = if limit_type == "movetime" {
        Some(limit as u64)
    } else {
        None
    };
    let nodes_limit = if limit_type == "nodes" {
        Some(limit as u64)
    } else {
        None
    };

    let start = Instant::now();
    let mut total_nodes: u64 = 0;

    for fen in &fens {
        let pos = match parse_fen(fen) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("bench: skipping invalid FEN '{fen}': {e}");
                continue;
            }
        };
        let limits = SearchLimits {
            depth: depth_limit,
            nodes: nodes_limit,
            ..Default::default()
        };
        let tc = if let Some(ms) = movetime_limit {
            Arc::new(TimeControl::for_movetime(ms))
        } else {
            Arc::new(TimeControl::none())
        };
        let stop = AtomicBool::new(false);
        // Generation bump once per position (matches the old per-search bump
        // inside `search()`; keeps the single-thread bench signature identical).
        tt.new_search();
        // Run search; nodes are accumulated in the global counter, returned in result.
        let result = crate::search::worker::search(
            &pos,
            limits,
            &stop,
            &tc,
            &tt,
            threads as u32,
            &mut |_event| {
                // Suppress per-iteration info — bench is quiet.
            },
        );
        total_nodes += result.nodes;
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;
    #[allow(clippy::manual_checked_ops)]
    let nps = if elapsed_ms > 0 {
        total_nodes * 1000 / elapsed_ms
    } else {
        total_nodes
    };

    // Stockfish-style summary block (stdout, not stderr).
    println!("===========================");
    println!("Total time (ms) : {}", elapsed_ms);
    println!("Nodes searched  : {}", total_nodes);
    println!("Nodes/second    : {}", nps);

    BenchStats {
        nodes: total_nodes,
        time_ms: elapsed_ms,
        nps,
    }
}

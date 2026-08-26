//! Lazy SMP — shared TT, per-thread search contexts.
//!
//! # Why lazy SMP works
//!
//! Lazy SMP is the simplest effective parallel search: all threads search the
//! same root position with iterative deepening and share a single lock-free
//! transposition table. No work splitting, no explicit synchronization — the
//! TT is the communication channel. When a helper finishes a subtree, the main
//! thread's later probe hits the cached score/best-move and cuts off the same
//! subtree, saving the re-search. The "lazy" in the name is literal: helpers
//! are not assigned work; they just race on the TT.
//!
//! Scaling is sublinear (e.g. 4 threads ≈ 2.5× NPS) because the tree shape is
//! inherently sequential (alpha-beta pruning depends on move ordering) and
//! because memory bandwidth is shared. Depth-offset helpers give a
//! small extra gain by starting at different depths so they probe different
//! subtrees early.
//!
//! ```text
//! GUI ── go ─▶ UciSession::handle_go ──► worker::search(threads=N) ──┐
//!                                    │  main worker (this thread)    │
//!                                    │  helpers (N-1 OS threads)     │
//!                                    │  shared: TT, stop, TimeControl│
//!                                    │  per-thread: killers/history/ │
//!                                    │    pawn cache, PV, nodes      │
//!                                    └───────────────────────────────┘
//! ```
//!
//! Only the main worker emits `SearchEvent`s (info lines, currmove); helpers
//! run silently with a no-op callback. Their `SearchResult` is discarded —
//! only their TT writes and aggregated node count matter. The final
//! `SearchResult.nodes` is the global total across all workers.
//!
//! # Node aggregation
//!
//! Each worker maintains a local `nodes` counter (plain `u64`, no atomics) and
//! periodically flushes its delta into a shared `AtomicU64` at the existing
//! 2048-node stop-check cadence. The `go nodes` limit and `info nodes/nps`
//! fields use the global total; overshoot is bounded by `threads × 2048`.
//!
//! # Thread lifetimes
//!
//! Helpers are scoped threads (`std::thread::scope`) so they can borrow the
//! shared `stop` flag without requiring `'static`. The main worker runs inline
//! on the calling thread (the session's supervisor thread / bench thread) so
//! the existing ponder-park and bestmove logic in `session.rs` stays unchanged.
//! After the main worker returns, `stop` is set to interrupt helpers promptly
//! and the scope joins them.
//!
//! See chessprogramming.org "Lazy SMP" and Joona Kiiski's lazy SMP notes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::board::Position;
use crate::search::{
    SearchEvent, SearchLimits, SearchResult, time::TimeControl, tt::TranspositionTable,
};

/// Run iterative deepening with `threads` workers.
///
/// `pos` is cloned per worker (the board is mutated via make/unmake inside
/// each worker). Only the main worker (thread 0) calls `on_event`; helpers
/// use a no-op callback. Returns the main worker's result with
/// `nodes` equal to the global total.
#[allow(dead_code)]
pub fn search(
    pos: &Position,
    limits: SearchLimits,
    stop: &AtomicBool,
    tc: &Arc<TimeControl>,
    tt: &Arc<TranspositionTable>,
    threads: u32,
    on_event: &mut dyn FnMut(SearchEvent),
) -> SearchResult {
    let threads = (threads.max(1) as usize).min(1024);
    let total_nodes = Arc::new(AtomicU64::new(0));

    if threads == 1 {
        let mut pos_clone = pos.clone();
        return crate::search::search_single(
            &mut pos_clone,
            limits,
            stop,
            tc,
            tt,
            &total_nodes,
            0,
            true,
            1,
            on_event,
        );
    }

    // Multi-threaded: main inline + helpers via scoped threads.
    // Helpers get multipv=1 when the main is in MultiPV mode — they just fill TT.
    let helper_limits = if limits.multipv > 1 {
        let mut l = limits.clone();
        l.multipv = 1;
        l
    } else {
        limits.clone()
    };

    std::thread::scope(|s| {
        // Spawn helpers (thread_id 1..threads-1) with depth offsets to de-correlate early iterations.
        for tid in 1..threads {
            let pos_c = pos.clone();
            let limits_c = helper_limits.clone();
            let tc_c = Arc::clone(tc);
            let tt_c = Arc::clone(tt);
            let total_c = Arc::clone(&total_nodes);
            let helper_depth = (1 + (tid % 2)) as u8;
            // `stop` is borrowed from the outer scope — scoped threads allow this.
            s.spawn(move || {
                let mut pos_c = pos_c;
                let mut dummy = |_: SearchEvent| {};
                crate::search::search_single(
                    &mut pos_c, limits_c, stop, &tc_c, &tt_c, &total_c, tid, false, helper_depth, &mut dummy,
                );
            });
        }

        // Main worker runs on this thread.
        let mut pos_main = pos.clone();
        let result = crate::search::search_single(
            &mut pos_main,
            limits,
            stop,
            tc,
            tt,
            &total_nodes,
            0,
            true,
            1,
            on_event,
        );

        // Interrupt helpers promptly; scoped join happens on drop.
        stop.store(true, Ordering::Relaxed);
        result
    })
}

// ---------------------------------------------------------------------------
// Tests — pool-level
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::parse_fen;
    use crate::search::MATE;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    fn search_pos(fen: &str, depth: u8, threads: u32) -> SearchResult {
        let pos = parse_fen(fen).unwrap();
        let limits = SearchLimits {
            depth: Some(depth),
            ..Default::default()
        };
        let stop = AtomicBool::new(false);
        let tc = Arc::new(TimeControl::none());
        let tt = Arc::new(TranspositionTable::new(1));
        let mut events = Vec::new();
        search(&pos, limits, &stop, &tc, &tt, threads, &mut |e| {
            events.push(e)
        })
    }

    #[test]
    fn single_thread_still_finds_mate() {
        let fen = "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 1 3";
        let res = search_pos(fen, 3, 1);
        assert!(
            res.score > MATE - 10,
            "expected mate score, got {}",
            res.score
        );
        assert!(res.best_move.is_some());
    }

    #[test]
    fn multi_thread_finds_mate() {
        let fen = "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 1 3";
        let res = search_pos(fen, 3, 4);
        assert!(
            res.score > MATE - 10,
            "expected mate score, got {}",
            res.score
        );
        assert!(res.best_move.is_some());
        // PV must be legal
        let mut pos = parse_fen(fen).unwrap();
        for mv in &res.pv {
            let mut moves = Vec::new();
            crate::board::generate_legal(&pos, &mut moves);
            assert!(moves.contains(mv), "pv move {mv} illegal");
            pos.make_move(*mv);
        }
    }

    #[test]
    fn pv_legal_multithreaded() {
        let fen = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
        let res = search_pos(fen, 3, 4);
        assert!(res.best_move.is_some());
        let mut pos = parse_fen(fen).unwrap();
        for mv in &res.pv {
            let mut moves = Vec::new();
            crate::board::generate_legal(&pos, &mut moves);
            assert!(moves.contains(mv), "pv move {mv} illegal");
            pos.make_move(*mv);
        }
    }

    #[test]
    fn nodes_limit_respected_mt() {
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let limits = SearchLimits {
            nodes: Some(5000),
            ..Default::default()
        };
        let stop = AtomicBool::new(false);
        let tc = Arc::new(TimeControl::none());
        let tt = Arc::new(TranspositionTable::new(1));
        let res = search(&pos, limits, &stop, &tc, &tt, 4, &mut |_| {});
        // Total nodes may overshoot by at most threads*2048 due to flush cadence.
        let slack = 4 * 2048 + 1000;
        assert!(
            res.nodes <= 5000 + slack,
            "nodes {} exceeds limit 5000 + slack {}",
            res.nodes,
            slack
        );
        assert!(res.nodes > 0);
    }

    #[test]
    fn stop_prompt_mt() {
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let limits = SearchLimits {
            infinite: true,
            ..Default::default()
        };
        let stop = Arc::new(AtomicBool::new(false));
        let tc = Arc::new(TimeControl::none());
        let tt = Arc::new(TranspositionTable::new(1));
        let stop_clone = Arc::clone(&stop);
        // Stop after a short delay from another thread.
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            stop_clone.store(true, Ordering::Relaxed);
        });
        let start = std::time::Instant::now();
        let res = search(&pos, limits, &stop, &tc, &tt, 4, &mut |_| {});
        let elapsed = start.elapsed();
        let _ = handle.join();
        // Search should have returned promptly (well under 5s).
        assert!(
            elapsed < Duration::from_secs(3),
            "stop not prompt: {elapsed:?}"
        );
        // Best move may be None if stopped before any iteration completed, but
        // with 80ms it should have found something.
        let _ = res.best_move;
        assert!(res.nodes > 0);
    }

    #[test]
    fn single_thread_deterministic_depth() {
        // Two runs at same depth with threads=1 must give identical best move
        // and score (TT is fresh each call, so determinism holds).
        let fen = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
        let r1 = search_pos(fen, 4, 1);
        let r2 = search_pos(fen, 4, 1);
        assert_eq!(r1.best_move, r2.best_move);
        assert_eq!(r1.score, r2.score);
    }
}

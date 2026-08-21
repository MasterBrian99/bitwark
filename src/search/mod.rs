//! Search — negamax alpha-beta, quiescence, iterative deepening.
//!
//! The full synchronous search runs on its own OS thread.
//! The search is pure synchronous CPU code; the UCI session stays on the
//! async runtime and talks to the search via an atomic stop flag and a
//! channel for `info`/`bestmove` lines.
//!
//! Lazy SMP: a main worker plus N-1 helper threads share a
//! single lock-free TT. Each worker owns its killers/history/pawn-cache/PV
//! via `SearchContext`; the global node count is aggregated through a shared
//! `AtomicU64` flushed at the existing 2048-node cadence.
//!
//! Architecture:
//! ```text
//! GUI ── go ─▶ UciSession::handle_go ──► worker::search(threads=N) ──┐
//!                                    │  main worker (this thread)    │
//!                                    │  helpers (N-1 OS threads)     │
//!                                    │  shared: TT, stop, TimeControl│
//!                                    └───────────────────────────────┘
//! ```

#![allow(clippy::collapsible_if)]
#![allow(clippy::manual_checked_ops)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::board::{Move, Position, generate_legal, is_square_attacked};
use crate::eval::PawnCache;

pub mod alpha_beta;
pub mod order;
pub mod quiescence;
pub mod time;
pub mod tt;
pub mod worker;

use time::TimeControl;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Mate score — larger than any possible eval.  `MATE - ply` means closer
/// mates are better.
pub const MATE: i32 = 30_000;
/// Rough infinity for alpha/beta initialization.
pub const VALUE_INFINITE: i32 = 32_000;
/// Maximum search depth (plies) including quiescence.
pub const MAX_PLY: usize = 128;

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

/// What the search is allowed to do.  Built from `GoParams` by the UCI
/// session; the search itself only sees this struct.
///
/// Time limits live in the shared `TimeControl` (soft/hard, pondering),
/// not here — `SearchLimits` holds the discrete counters/flags.
#[derive(Debug, Clone)]
pub struct SearchLimits {
    /// Nominal depth limit, if any.
    pub depth: Option<u8>,
    /// Approximate node-count limit, if any.
    pub nodes: Option<u64>,
    /// Search for mate in this many moves, if any.
    pub mate: Option<u8>,
    /// Search forever until `stop` (uci `go infinite`).
    pub infinite: bool,
    /// Pondering mode (`go ponder`); search doesn't stop on mate/time.
    pub ponder: bool,
    /// Number of principal variations (MultiPV).
    #[allow(dead_code)]
    pub multipv: u32,
    /// Restrict root to these moves (empty = unrestricted).
    pub searchmoves: Vec<Move>,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            depth: None,
            nodes: None,
            mate: None,
            infinite: false,
            ponder: false,
            multipv: 1,
            searchmoves: Vec::new(),
        }
    }
}

impl SearchLimits {
    /// Maximum nominal depth for iterative deepening.
    ///
    /// When a time control is active (`tc` has limits) we allow deep search
    /// up to `MAX_PLY` — the time manager stops us. Bare `go` (no depth, no
    /// mate, no infinite, no time) is the implicit depth 245 per UCI spec §2.8.
    pub fn max_depth(&self, tc: &TimeControl) -> usize {
        if let Some(m) = self.mate {
            return (m as usize * 2).min(MAX_PLY);
        }
        if let Some(d) = self.depth {
            return (d as usize).min(MAX_PLY);
        }
        if self.infinite {
            return MAX_PLY;
        }
        // Time-limited go (soft or hard > 0) → deep, stopped by Tc.
        if tc.soft_ms() > 0 || tc.hard_ms() > 0 {
            return MAX_PLY;
        }
        // Bare `go` — UCI spec §2.8: treat as very deep (Stockfish 245).
        245.min(MAX_PLY)
    }
}

// ---------------------------------------------------------------------------
// Search events — streamed to the session for `info` line formatting
// ---------------------------------------------------------------------------

/// Events streamed from the search thread to the UCI session.
///
/// `Iteration` = one completed depth (the `info depth … pv …` line).
/// `CurrMove`  = root move currently being searched (`currmove`/`currmovenumber`).
#[derive(Debug, Clone)]
pub enum SearchEvent {
    Iteration {
        depth: u8,
        seldepth: usize,
        multipv: u32,
        score: i32,
        bound: Option<tt::Bound>,
        nodes: u64,
        time_ms: u64,
        nps: u64,
        hashfull: u32,
        pv: Vec<Move>,
    },
    CurrMove {
        depth: u8,
        currmove: Move,
        number: usize,
    },
}

// ---------------------------------------------------------------------------
// Search result (final bestmove)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SearchResult {
    pub best_move: Option<Move>,
    pub score: i32,
    pub nodes: u64,
    pub pv: Vec<Move>,
}

// ---------------------------------------------------------------------------
// Search context — shared mutable state threaded through recursion
// ---------------------------------------------------------------------------

pub struct SearchContext<'a> {
    pub nodes: u64,
    pub seldepth: usize,
    pub pv_table: [[Option<Move>; MAX_PLY]; MAX_PLY],
    pub pv_len: [usize; MAX_PLY],
    pub stop: &'a AtomicBool,
    pub limits: SearchLimits,
    pub tc: Arc<TimeControl>,
    /// Shared TT (SMP-ready, lock-free).
    pub tt: Arc<tt::TranspositionTable>,
    /// Killers: two quiet moves per ply that caused a beta cutoff.
    #[allow(dead_code)]
    pub killers: [[Option<Move>; 2]; MAX_PLY],
    /// History: side × from × to, gravity with cap ±16384.
    #[allow(dead_code)]
    pub history: Box<[[[i32; 64]; 64]; 2]>,
    /// Pawn structure cache (direct-mapped).
    pub pawn_cache: PawnCache,
    /// Global node counter shared across SMP workers.
    pub total_nodes: Arc<AtomicU64>,
    /// Last flushed local node count (for delta flushing).
    pub last_flush: u64,
    /// Worker index (0 = main).
    #[allow(dead_code)]
    pub thread_id: usize,
    /// Whether this worker should emit `SearchEvent`s.
    pub is_main: bool,
}

impl<'a> SearchContext<'a> {
    #[allow(dead_code)]
    pub fn new(
        stop: &'a AtomicBool,
        limits: SearchLimits,
        tc: Arc<TimeControl>,
        tt: Arc<tt::TranspositionTable>,
    ) -> Self {
        Self::new_with_nodes(stop, limits, tc, tt, Arc::new(AtomicU64::new(0)), 0, true)
    }

    pub fn new_with_nodes(
        stop: &'a AtomicBool,
        limits: SearchLimits,
        tc: Arc<TimeControl>,
        tt: Arc<tt::TranspositionTable>,
        total_nodes: Arc<AtomicU64>,
        thread_id: usize,
        is_main: bool,
    ) -> Self {
        Self {
            nodes: 0,
            seldepth: 0,
            pv_table: [[None; MAX_PLY]; MAX_PLY],
            pv_len: [0; MAX_PLY],
            stop,
            limits,
            tc,
            tt,
            killers: [[None; 2]; MAX_PLY],
            history: Box::new([[[0; 64]; 64]; 2]),
            pawn_cache: PawnCache::new(),
            total_nodes,
            last_flush: 0,
            thread_id,
            is_main,
        }
    }

    /// Flush local delta into the global counter.
    #[inline]
    pub fn flush_nodes(&mut self) {
        let delta = self.nodes.wrapping_sub(self.last_flush);
        if delta > 0 {
            self.total_nodes.fetch_add(delta, Ordering::Relaxed);
            self.last_flush = self.nodes;
        }
    }

    /// Global total including unflushed local delta (for event reporting).
    #[inline]
    pub fn global_nodes(&self) -> u64 {
        let unflushed = self.nodes.wrapping_sub(self.last_flush);
        self.total_nodes
            .load(Ordering::Relaxed)
            .wrapping_add(unflushed)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn filtered_root_moves(pos: &Position, searchmoves: &[Move]) -> Vec<Move> {
    let mut moves = Vec::new();
    generate_legal(pos, &mut moves);
    if !searchmoves.is_empty() {
        let original = moves.clone();
        moves.retain(|m| searchmoves.contains(m));
        if moves.is_empty() {
            moves = original;
        }
    }
    moves
}

// ---------------------------------------------------------------------------
// Public entry — iterative deepening (single-thread compat wrapper)
// ---------------------------------------------------------------------------

/// Run iterative deepening on `pos` until a limit or `stop`.
///
/// Calls `on_event` once per completed depth.  Returns the best move found
/// (or `None` if the root has no legal moves).  The position is left
/// unmodified (all make/unmake balanced).
///
/// This is the single-thread compat entry used by tests. Production `go`
/// and `bench` go through `worker::search` (which adds SMP).
#[allow(dead_code)]
pub fn search(
    pos: &mut Position,
    limits: SearchLimits,
    stop: &AtomicBool,
    tc: &Arc<TimeControl>,
    tt: &Arc<tt::TranspositionTable>,
    on_event: &mut dyn FnMut(SearchEvent),
) -> SearchResult {
    let total = Arc::new(AtomicU64::new(0));
    search_single(pos, limits, stop, tc, tt, &total, 0, true, 1, on_event)
}

/// Core iterative-deepening implementation, parameterized for SMP.
///
/// `thread_id` 0 is the main worker; `is_main` controls event emission.
/// `start_depth` lets helpers begin at a deeper depth (the Lazy SMP depth offset).
#[allow(clippy::too_many_arguments)]
pub(crate) fn search_single(
    pos: &mut Position,
    limits: SearchLimits,
    stop: &AtomicBool,
    tc: &Arc<TimeControl>,
    tt: &Arc<tt::TranspositionTable>,
    total_nodes: &Arc<AtomicU64>,
    thread_id: usize,
    is_main: bool,
    start_depth: u8,
    on_event: &mut dyn FnMut(SearchEvent),
) -> SearchResult {
    let max_depth = limits.max_depth(tc);
    let start_depth = (start_depth as usize).max(1).min(max_depth.max(1));
    let mut ctx = SearchContext::new_with_nodes(
        stop,
        limits.clone(),
        Arc::clone(tc),
        Arc::clone(tt),
        Arc::clone(total_nodes),
        thread_id,
        is_main,
    );

    // Helpers in MultiPV mode run single-PV (just fill TT).
    if !is_main && ctx.limits.multipv > 1 {
        ctx.limits.multipv = 1;
    }

    let root_moves = filtered_root_moves(pos, &ctx.limits.searchmoves);
    if root_moves.is_empty() {
        let score = if let Some(king_sq) = pos.king_square(pos.side_to_move()) {
            if is_square_attacked(pos, king_sq, pos.side_to_move().opposite()) {
                -MATE
            } else {
                0
            }
        } else {
            0
        };
        return SearchResult {
            best_move: None,
            score,
            nodes: 0,
            pv: Vec::new(),
        };
    }

    let mut best_move = root_moves[0];
    let mut best_score: i32 = 0;
    let mut best_pv: Vec<Move> = vec![best_move];
    let mut prev_score: i32 = 0;
    let mut last_iter_ms: u64 = 0;
    let mut have_prev = false;

    // Iterative deepening with aspiration windows.
    for depth in start_depth..=max_depth {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if ctx.tc.should_hard_stop() {
            stop.store(true, Ordering::Relaxed);
            break;
        }
        if ctx.tc.should_soft_stop() {
            break;
        }
        if depth > start_depth
            && ctx.tc.hard_ms() > 0
            && ctx.tc.soft_ms() != ctx.tc.hard_ms()
            && last_iter_ms > 0
        {
            let elapsed = ctx.tc.elapsed_ms();
            if elapsed + 2 * last_iter_ms > ctx.tc.hard_ms() {
                break;
            }
        }
        if depth > start_depth
            && !ctx.limits.ponder
            && !ctx.limits.infinite
            && ctx.limits.depth.is_none()
            && ctx.limits.mate.is_none()
            && ctx.tc.soft_ms() > 0
            && prev_score.abs() >= MATE - MAX_PLY as i32
        {
            break;
        }
        if let Some(mate_n) = ctx.limits.mate {
            if prev_score >= MATE - 2 * mate_n as i32 {
                break;
            }
        }

        // MultiPV: only the main worker emits multiple lines.
        if ctx.limits.multipv > 1 && is_main {
            let moves = filtered_root_moves(pos, &ctx.limits.searchmoves);
            if moves.is_empty() {
                break;
            }
            let mut results: Vec<(i32, Move, Vec<Move>)> = Vec::new();
            let mut aborted = false;
            let mut last_curr_emit_ms: u64 = 0;
            for (idx, &mv) in moves.iter().enumerate() {
                if is_main {
                    let now = ctx.tc.elapsed_ms();
                    let do_emit = depth <= 1 || now.saturating_sub(last_curr_emit_ms) >= 100;
                    if do_emit {
                        last_curr_emit_ms = now;
                        on_event(SearchEvent::CurrMove {
                            depth: depth as u8,
                            currmove: mv,
                            number: idx + 1,
                        });
                    }
                }
                if stop.load(Ordering::Relaxed) {
                    aborted = true;
                    break;
                }
                if ctx.tc.should_hard_stop() {
                    stop.store(true, Ordering::Relaxed);
                    aborted = true;
                    break;
                }
                if let Some(limit) = ctx.limits.nodes {
                    let total = ctx.global_nodes();
                    if total >= limit {
                        stop.store(true, Ordering::Relaxed);
                        aborted = true;
                        break;
                    }
                }
                ctx.pv_len[1] = 0;
                pos.make_move(mv);
                let score = -alpha_beta::negamax(
                    pos,
                    depth as i32 - 1,
                    -VALUE_INFINITE,
                    VALUE_INFINITE,
                    1,
                    true,
                    &mut ctx,
                );
                pos.unmake_move(mv);
                if stop.load(Ordering::Relaxed) {
                    aborted = true;
                    break;
                }
                let mut pv = Vec::new();
                pv.push(mv);
                let child_len = ctx.pv_len[1];
                for j in 0..child_len {
                    if let Some(m) = ctx.pv_table[1][j] {
                        pv.push(m);
                    }
                }
                results.push((score, mv, pv));
            }
            if aborted {
                if stop.load(Ordering::Relaxed) && !best_pv.is_empty() && is_main {
                    ctx.flush_nodes();
                    let total = ctx.total_nodes.load(Ordering::Relaxed);
                    let elapsed = ctx.tc.elapsed_ms();
                    let nps = if elapsed > 0 {
                        total * 1000 / elapsed
                    } else {
                        total
                    };
                    on_event(SearchEvent::Iteration {
                        depth: (depth as u8).saturating_sub(1),
                        seldepth: ctx.seldepth,
                        multipv: 1,
                        score: best_score,
                        bound: None,
                        nodes: total,
                        time_ms: elapsed,
                        nps,
                        hashfull: ctx.tt.hashfull_permille(),
                        pv: best_pv.clone(),
                    });
                }
                break;
            }
            results.sort_by_key(|r| std::cmp::Reverse(r.0));
            let k = (ctx.limits.multipv as usize).min(results.len());
            best_move = results[0].1;
            best_score = results[0].0;
            best_pv = results[0].2.clone();
            prev_score = best_score;
            have_prev = true;
            ctx.flush_nodes();
            let total = ctx.total_nodes.load(Ordering::Relaxed);
            let elapsed = ctx.tc.elapsed_ms();
            let nps = if elapsed > 0 {
                total * 1000 / elapsed
            } else {
                total
            };
            last_iter_ms = elapsed;
            if is_main {
                for (idx, (score, _mv, pv)) in results.iter().enumerate().take(k) {
                    on_event(SearchEvent::Iteration {
                        depth: depth as u8,
                        seldepth: ctx.seldepth,
                        multipv: (idx + 1) as u32,
                        score: *score,
                        bound: None,
                        nodes: total,
                        time_ms: elapsed,
                        nps,
                        hashfull: ctx.tt.hashfull_permille(),
                        pv: pv.clone(),
                    });
                }
            }
            continue;
        } else if ctx.limits.multipv > 1 && !is_main {
            // Helpers shouldn't be in MultiPV — they were forced to single-PV above,
            // but if we reach here with multipv>1 and not main, skip the iteration.
            // This branch is unreachable due to the force above, but keep for safety.
            continue;
        }

        // Aspiration window (depth >=5 and not a mate score).
        let mut delta: i32 = 25;
        let mut alpha = -VALUE_INFINITE;
        let mut beta = VALUE_INFINITE;
        if depth >= 5 && have_prev && prev_score.abs() < MATE - MAX_PLY as i32 {
            alpha = prev_score - delta;
            beta = prev_score + delta;
        }

        let mut iteration_best: Option<(Move, i32)> = None;
        loop {
            let res = root_search(pos, depth as i32, alpha, beta, &mut ctx, on_event);
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let Some((mv, score)) = res else {
                break;
            };
            if score <= alpha {
                if is_main {
                    ctx.flush_nodes();
                    let total = ctx.total_nodes.load(Ordering::Relaxed);
                    let elapsed = ctx.tc.elapsed_ms();
                    let nps = if elapsed > 0 {
                        total * 1000 / elapsed
                    } else {
                        total
                    };
                    on_event(SearchEvent::Iteration {
                        depth: depth as u8,
                        seldepth: ctx.seldepth,
                        multipv: 1,
                        score,
                        bound: Some(tt::Bound::Upper),
                        nodes: total,
                        time_ms: elapsed,
                        nps,
                        hashfull: ctx.tt.hashfull_permille(),
                        pv: Vec::new(),
                    });
                }
                beta = (alpha + beta) / 2;
                alpha -= delta;
                delta += delta / 2;
                if delta > 500 {
                    alpha = -VALUE_INFINITE;
                    beta = VALUE_INFINITE;
                }
                continue;
            } else if score >= beta {
                if is_main {
                    ctx.flush_nodes();
                    let total = ctx.total_nodes.load(Ordering::Relaxed);
                    let elapsed = ctx.tc.elapsed_ms();
                    let nps = if elapsed > 0 {
                        total * 1000 / elapsed
                    } else {
                        total
                    };
                    on_event(SearchEvent::Iteration {
                        depth: depth as u8,
                        seldepth: ctx.seldepth,
                        multipv: 1,
                        score,
                        bound: Some(tt::Bound::Lower),
                        nodes: total,
                        time_ms: elapsed,
                        nps,
                        hashfull: ctx.tt.hashfull_permille(),
                        pv: Vec::new(),
                    });
                }
                beta += delta;
                delta += delta / 2;
                if delta > 500 {
                    alpha = -VALUE_INFINITE;
                    beta = VALUE_INFINITE;
                }
                continue;
            } else {
                iteration_best = Some((mv, score));
                prev_score = score;
                have_prev = true;
                break;
            }
        }

        if stop.load(Ordering::Relaxed) {
            if !best_pv.is_empty() && is_main {
                ctx.flush_nodes();
                let total = ctx.total_nodes.load(Ordering::Relaxed);
                let elapsed = ctx.tc.elapsed_ms();
                let nps = if elapsed > 0 {
                    total * 1000 / elapsed
                } else {
                    total
                };
                on_event(SearchEvent::Iteration {
                    depth: (depth as u8).saturating_sub(1),
                    seldepth: ctx.seldepth,
                    multipv: 1,
                    score: best_score,
                    bound: None,
                    nodes: total,
                    time_ms: elapsed,
                    nps,
                    hashfull: ctx.tt.hashfull_permille(),
                    pv: best_pv.clone(),
                });
            }
            break;
        }
        let Some((mv, score)) = iteration_best else {
            continue;
        };

        let pv_len = ctx.pv_len[0];
        let mut pv = Vec::with_capacity(pv_len);
        for i in 0..pv_len {
            if let Some(m) = ctx.pv_table[0][i] {
                pv.push(m);
            }
        }
        if pv.is_empty() {
            pv.push(mv);
        }

        best_move = mv;
        best_score = score;
        best_pv = pv.clone();

        ctx.flush_nodes();
        let total = ctx.total_nodes.load(Ordering::Relaxed);
        let elapsed_ms = ctx.tc.elapsed_ms();
        let nps = if elapsed_ms > 0 {
            total * 1000 / elapsed_ms
        } else {
            total
        };
        last_iter_ms = elapsed_ms;

        if is_main {
            on_event(SearchEvent::Iteration {
                depth: depth as u8,
                seldepth: ctx.seldepth,
                multipv: 1,
                score,
                bound: None,
                nodes: total,
                time_ms: elapsed_ms,
                nps,
                hashfull: ctx.tt.hashfull_permille(),
                pv: pv.clone(),
            });
        }

        let _ = best_score;
    }

    // Final flush so the global total is accurate.
    ctx.flush_nodes();
    let total = ctx.total_nodes.load(Ordering::Relaxed);
    SearchResult {
        best_move: Some(best_move),
        score: best_score,
        nodes: total,
        pv: best_pv,
    }
}

/// One iterative-deepening iteration at the root.
///
/// Generates root moves, puts the previous iteration's best first, then
/// searches each move with `negamax` using PVS. Returns the best move and
/// its score, or `None` if the search was aborted mid-iteration.
/// The window `[alpha, beta]` is the aspiration window from the ID loop.
fn root_search(
    pos: &mut Position,
    depth: i32,
    mut alpha: i32,
    beta: i32,
    ctx: &mut SearchContext,
    on_event: &mut dyn FnMut(SearchEvent),
) -> Option<(Move, i32)> {
    let mut moves = filtered_root_moves(pos, &ctx.limits.searchmoves);
    if moves.is_empty() {
        return None;
    }

    // Previous iteration's PV move first (if any).
    if ctx.pv_len[0] > 0
        && let Some(prev_best) = ctx.pv_table[0][0]
        && let Some(idx) = moves.iter().position(|&m| m == prev_best)
    {
        moves.swap(0, idx);
    }

    let mut best_move = moves[0];
    let mut best_score = -VALUE_INFINITE;
    let orig_alpha = alpha;

    let _prev_best_cached = if ctx.pv_len[0] > 0 {
        ctx.pv_table[0][0]
    } else {
        None
    };

    let mut last_curr_emit_ms: u64 = 0;
    for i in 0..moves.len() {
        if i > 0 {
            let best_idx = {
                let slice = &moves[i..];
                let mut best = 0;
                let mut best_s =
                    order::score_move(pos, slice[0], None, &ctx.killers[0], &ctx.history, 0);
                for (j, &mv) in slice.iter().enumerate().skip(1) {
                    let s = order::score_move(pos, mv, None, &ctx.killers[0], &ctx.history, 0);
                    if s > best_s {
                        best_s = s;
                        best = j;
                    }
                }
                best
            };
            moves.swap(i, i + best_idx);
        }
        let mv = moves[i];

        // currmove / currmovenumber — UCI spec §3.4 (main only)
        if ctx.is_main {
            let now = ctx.tc.elapsed_ms();
            let do_emit = depth <= 1 || now.saturating_sub(last_curr_emit_ms) >= 100;
            if do_emit {
                last_curr_emit_ms = now;
                on_event(SearchEvent::CurrMove {
                    depth: depth as u8,
                    currmove: mv,
                    number: i + 1,
                });
            }
        }

        if ctx.stop.load(Ordering::Relaxed) {
            return None;
        }
        if ctx.tc.should_hard_stop() {
            ctx.stop.store(true, Ordering::Relaxed);
            return None;
        }
        // Periodic flush + checks at root per-move (also handled inside negamax).
        if ctx.nodes.is_multiple_of(2048) {
            ctx.flush_nodes();
            if ctx.tc.should_hard_stop() {
                ctx.stop.store(true, Ordering::Relaxed);
                return None;
            }
            if let Some(limit) = ctx.limits.nodes {
                if ctx.total_nodes.load(Ordering::Relaxed) >= limit {
                    ctx.stop.store(true, Ordering::Relaxed);
                    return None;
                }
            }
        } else if let Some(limit) = ctx.limits.nodes {
            // Also check total periodically even without flush (other threads may have filled it).
            if ctx.total_nodes.load(Ordering::Relaxed) >= limit {
                ctx.stop.store(true, Ordering::Relaxed);
                return None;
            }
        }

        pos.make_move(mv);
        let score = if i == 0 {
            -alpha_beta::negamax(pos, depth - 1, -beta, -alpha, 1, true, ctx)
        } else {
            // PVS zero-window
            let v = -alpha_beta::negamax(pos, depth - 1, -alpha - 1, -alpha, 1, true, ctx);
            if v > alpha && v < beta {
                // Re-search with full window (PV node)
                -alpha_beta::negamax(pos, depth - 1, -beta, -alpha, 1, true, ctx)
            } else {
                v
            }
        };
        pos.unmake_move(mv);

        if ctx.stop.load(Ordering::Relaxed) {
            return None;
        }

        if score > best_score {
            best_score = score;
            best_move = mv;
        }
        if score > alpha {
            alpha = score;
            ctx.pv_table[0][0] = Some(mv);
            let child_len = ctx.pv_len[1];
            for j in 0..child_len {
                ctx.pv_table[0][j + 1] = ctx.pv_table[1][j];
            }
            ctx.pv_len[0] = child_len + 1;
        }
        let _ = orig_alpha;
    }

    Some((best_move, best_score))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::parse_fen;
    use std::sync::atomic::AtomicBool;

    fn search_depth(fen: &str, depth: u8) -> SearchResult {
        let mut pos = parse_fen(fen).unwrap();
        let limits = SearchLimits {
            depth: Some(depth),
            ..Default::default()
        };
        let stop = AtomicBool::new(false);
        let tc = Arc::new(TimeControl::none());
        let tt = Arc::new(crate::search::tt::TranspositionTable::new(1));
        let mut events: Vec<SearchEvent> = Vec::new();
        let res = search(&mut pos, limits, &stop, &tc, &tt, &mut |e| events.push(e));
        let has_iter = events
            .iter()
            .any(|e| matches!(e, SearchEvent::Iteration { .. }));
        assert!(has_iter || depth == 0);
        res
    }

    #[test]
    fn mate_in_1_found() {
        let fen = "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 1 3";
        let res = search_depth(fen, 2);
        let mv_str = res.best_move.unwrap().to_string();
        let mut pos = parse_fen(fen).unwrap();
        pos.make_move(res.best_move.unwrap());
        let mut moves = Vec::new();
        crate::board::generate_legal(&mut pos, &mut moves);
        assert!(
            moves.is_empty(),
            "expected checkmate, but opponent has {} moves, best was {mv_str}",
            moves.len()
        );
        assert!(
            is_square_attacked(
                &pos,
                pos.king_square(pos.side_to_move()).unwrap(),
                pos.side_to_move().opposite()
            ),
            "expected in check, best was {mv_str}"
        );
    }

    #[test]
    fn mate_in_2_found() {
        let fen = "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 1 3";
        let res = search_depth(fen, 3);
        assert!(
            res.score > MATE - 10,
            "expected mate score, got {}",
            res.score
        );
    }

    #[test]
    fn repetition_draw() {
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let mut pos = parse_fen(fen).unwrap();
        let m1 = crate::board::mv::Move::parse_uci("g1f3").unwrap();
        let m2 = crate::board::mv::Move::parse_uci("g8f6").unwrap();
        let m3 = crate::board::mv::Move::parse_uci("f3g1").unwrap();
        let m4 = crate::board::mv::Move::parse_uci("f6g8").unwrap();
        pos.make_move(m1);
        pos.make_move(m2);
        pos.make_move(m3);
        pos.make_move(m4);
        assert!(
            pos.is_repetition(),
            "should be repetition after shuffling back"
        );
        let res = search_depth(&crate::board::fen::to_fen(&pos), 3);
        assert!(res.score.abs() < MATE / 2);
    }

    #[test]
    fn fifty_move_draw() {
        let fen = "4k3/8/8/8/8/8/8/4K3 w - - 100 50";
        let mut pos = parse_fen(fen).unwrap();
        assert!(pos.is_fifty_move_draw());
        let res = search_depth(fen, 2);
        assert_eq!(res.score, 0);
    }

    #[test]
    fn pv_legal() {
        let fen = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
        let mut pos = parse_fen(fen).unwrap();
        let limits = SearchLimits {
            depth: Some(3),
            ..Default::default()
        };
        let stop = AtomicBool::new(false);
        let tc = Arc::new(TimeControl::none());
        let tt = Arc::new(crate::search::tt::TranspositionTable::new(1));
        let mut pv = Vec::new();
        let _ = search(&mut pos, limits, &stop, &tc, &tt, &mut |e| {
            if let SearchEvent::Iteration { pv: p, .. } = e {
                pv = p.clone();
            }
        });
        let mut tmp = parse_fen(fen).unwrap();
        for mv in &pv {
            let mut moves = Vec::new();
            crate::board::generate_legal(&mut tmp, &mut moves);
            assert!(moves.contains(mv), "pv move {mv} illegal");
            tmp.make_move(*mv);
        }
    }
}

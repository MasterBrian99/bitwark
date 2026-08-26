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

use crate::board::types::{PieceType, Square};
use crate::board::{Move, Position, generate_legal, is_square_attacked};
use crate::eval::PawnCache;

pub type CounterTable = [[Option<Move>; 64]; 6];
pub type ContHistory = [[[[i32; 64]; 6]; 64]; 6];

pub mod alpha_beta;
pub mod order;
pub mod quiescence;
pub mod see;
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
/// Node interval for stop/time/node-limit checks (countdown in SearchContext).
pub const NODE_CHECK_INTERVAL: u32 = 2048;

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
    pub killers: [[Option<Move>; 2]; MAX_PLY],
    /// History: side × from × to, gravity with cap ±16384.
    pub history: Box<[[[i32; 64]; 64]; 2]>,
    /// Countermoves: one reply per [prev piece type][prev to] (10a).
    pub countermoves: Box<CounterTable>,
    /// Continuation history 1-ply and 2-ply (10b).
    pub cont1: Box<ContHistory>,
    pub cont2: Box<ContHistory>,
    /// Last move per ply: (piece type before move, to square) — for countermove
    /// and continuation history (10a/b). `None` after null move.
    pub prev_stack: [Option<(PieceType, Square)>; MAX_PLY],
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
    /// Countdown to next stop/time check (2048-node cadence, see `tick_check`).
    pub check_count: u32,
    /// Per-ply quiets tried at this node (for history malus, 1.4).
    /// Only `0..quiets_cnt[ply]` is valid; stale entries are never read.
    pub quiets_stack: Box<[[Option<Move>; 218]; MAX_PLY]>,
    /// Consecutive singular extensions along current line (SE cap, 3.2/3.3).
    pub se_extensions: [u8; MAX_PLY],
    /// Pawn correction history — mg/eg pair per (pawn_hash bucket, side to move).
    /// Bucket = pawn_hash & 16383, side = stm, [mg, eg]. Applied as
    /// `((mg*phase + eg*(24-phase))/24)/256`. 256 KiB per thread (4.2).
    pub pawn_corr: Box<[[[i32; 2]; 2]; 16384]>,
    /// Material correction history — same shape, keyed on material_hash (4.3).
    pub mat_corr: Box<[[[i32; 2]; 2]; 16384]>,
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
            countermoves: Box::new([[None; 64]; 6]),
            cont1: Box::new([[[[0; 64]; 6]; 64]; 6]),
            cont2: Box::new([[[[0; 64]; 6]; 64]; 6]),
            prev_stack: [None; MAX_PLY],
            pawn_cache: PawnCache::new(),
            total_nodes,
            last_flush: 0,
            thread_id,
            is_main,
            check_count: 1,
            quiets_stack: Box::new([[None; 218]; MAX_PLY]),
            se_extensions: [0; MAX_PLY],
            pawn_corr: Box::new([[[0; 2]; 2]; 16384]),
            mat_corr: Box::new([[[0; 2]; 2]; 16384]),
        }
    }

    /// Countdown tick for stop/time checks. Returns true every `NODE_CHECK_INTERVAL` calls.
    /// Initialized to 1 so the first call fires (matches `nodes == 0` case).
    #[inline]
    pub fn tick_check(&mut self) -> bool {
        self.check_count = self.check_count.wrapping_sub(1);
        if self.check_count == 0 {
            self.check_count = NODE_CHECK_INTERVAL;
            true
        } else {
            false
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

impl<'a> SearchContext<'a> {
    #[inline]
    fn corr_gravity(entry: &mut i32, bonus: i32) {
        *entry += bonus - *entry * bonus / 16384;
        *entry = (*entry).clamp(-16384, 16384);
    }

    #[inline]
    pub fn pawn_correction(&self, pos: &Position) -> i32 {
        let phase = crate::eval::game_phase(pos);
        let bucket = (pos.pawn_hash() as usize) & 16383;
        let side = pos.side_to_move() as usize;
        let mg = self.pawn_corr[bucket][side][0];
        let eg = self.pawn_corr[bucket][side][1];
        let interp = (mg * phase + eg * (24 - phase)) / 24;
        interp / 256
    }

    #[inline]
    pub fn mat_correction(&self, _pos: &Position) -> i32 {
        // 4.3b will wire material_hash; stub returns 0 for 4.2
        0
    }

    #[inline]
    pub fn correction(&self, pos: &Position) -> i32 {
        self.pawn_correction(pos) + self.mat_correction(pos)
    }

    pub fn update_pawn_correction(&mut self, pos: &Position, bonus: i32) {
        let phase = crate::eval::game_phase(pos);
        let bucket = (pos.pawn_hash() as usize) & 16383;
        let side = pos.side_to_move() as usize;
        // Compensation so applied delta at observed phase equals bonus
        let denom = phase * phase + (24 - phase) * (24 - phase);
        let c = if denom == 0 { 1 } else { 576 / denom };
        let mg_bonus = bonus * phase / 24 * c;
        let eg_bonus = bonus * (24 - phase) / 24 * c;
        Self::corr_gravity(&mut self.pawn_corr[bucket][side][0], mg_bonus);
        Self::corr_gravity(&mut self.pawn_corr[bucket][side][1], eg_bonus);
    }

    pub fn update_mat_correction(&mut self, _pos: &Position, _bonus: i32) {
        // 4.3b will implement material correction; stub for 4.2
    }

    pub fn update_correction(&mut self, pos: &Position, bonus: i32) {
        self.update_pawn_correction(pos, bonus);
        self.update_mat_correction(pos, bonus);
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
        // Clear SE extension stack each iteration (otherwise stale counts leak across ID depths)
        ctx.se_extensions = [0; MAX_PLY];
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
                // Set prev_stack[0] for child ply 1 (10a)
                if let Some(pc) = pos.piece_at(mv.from) {
                    ctx.prev_stack[0] = Some((pc.piece_type(), mv.to));
                } else {
                    ctx.prev_stack[0] = None;
                }
                pos.make_move(mv);
                let score = -alpha_beta::negamax(
                    pos,
                    depth as i32 - 1,
                    -VALUE_INFINITE,
                    VALUE_INFINITE,
                    1,
                    true,
                    None,
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
                let mut best_s = order::score_move_with_see(
                    pos,
                    slice[0],
                    None,
                    &ctx.killers[0],
                    None,
                    &ctx.history,
                    None,
                    &crate::search::order::ZERO_CONT,
                    None,
                    &crate::search::order::ZERO_CONT,
                    0,
                );
                for (j, &mv) in slice.iter().enumerate().skip(1) {
                    let s = order::score_move_with_see(
                        pos,
                        mv,
                        None,
                        &ctx.killers[0],
                        None,
                        &ctx.history,
                        None,
                        &crate::search::order::ZERO_CONT,
                        None,
                        &crate::search::order::ZERO_CONT,
                        0,
                    );
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
        if ctx.tick_check() {
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

        // Record prev move for child ply=1 (counter/continuation — 10a)
        if let Some(pc) = pos.piece_at(mv.from) {
            ctx.prev_stack[0] = Some((pc.piece_type(), mv.to));
        } else {
            ctx.prev_stack[0] = None;
        }
        pos.make_move(mv);
        let score = if i == 0 {
            -alpha_beta::negamax(pos, depth - 1, -beta, -alpha, 1, true, None, ctx)
        } else {
            // PVS zero-window
            let v = -alpha_beta::negamax(pos, depth - 1, -alpha - 1, -alpha, 1, true, None, ctx);
            if v > alpha && v < beta {
                // Re-search with full window (PV node)
                -alpha_beta::negamax(pos, depth - 1, -beta, -alpha, 1, true, None, ctx)
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
    fn mate_in_5_found() {
        // Q2K4/8/3k4/8/8/8/8/8 w - - 0 1 is mate in 5 (Stockfish). Exercises singular extensions.
        let fen = "Q2K4/8/3k4/8/8/8/8/8 w - - 0 1";
        let res = search_depth(fen, 10);
        assert!(
            res.score > MATE - 20,
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

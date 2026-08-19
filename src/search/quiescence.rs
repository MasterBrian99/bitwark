//! Quiescence search — extend search until the position is quiet.
//!
//! Only captures and promotions are searched; quiet moves are ignored.
//! Stand-pat allows the side to move to "pass" if already good enough.
//! See chessprogramming.org "Quiescence Search".

#![allow(clippy::collapsible_if)]

use std::sync::atomic::Ordering;

use crate::board::{Position, generate_captures};
use crate::eval::evaluate;

use super::{MAX_PLY, SearchContext};

/// Quiescence negamax.
///
/// `alpha`/`beta` are the window, `ply` is distance from root (for mate
/// scoring and PV table).  Returns the best score from the side-to-move's
/// perspective.
pub fn quiescence(
    pos: &mut Position,
    mut alpha: i32,
    beta: i32,
    ply: usize,
    ctx: &mut SearchContext,
) -> i32 {
    // Stop check — cheap, done every node.
    if ctx.stop.load(Ordering::Relaxed) {
        return 0;
    }
    if ctx.tc.should_hard_stop() {
        ctx.stop.store(true, Ordering::Relaxed);
        return 0;
    }
    // Time check every 2048 nodes (amortised).
    if ctx.nodes.is_multiple_of(2048) && ctx.tc.should_hard_stop() {
        ctx.stop.store(true, Ordering::Relaxed);
        return 0;
    }
    if let Some(limit) = ctx.limits.nodes {
        if ctx.nodes >= limit {
            ctx.stop.store(true, Ordering::Relaxed);
            return 0;
        }
    }

    if ply >= MAX_PLY - 1 {
        return evaluate(pos);
    }

    ctx.nodes += 1;
    if ply > ctx.seldepth {
        ctx.seldepth = ply;
    }

    // Draw detection — if this position is already drawn, don't stand pat
    // higher than 0.
    if pos.is_repetition() || pos.is_fifty_move_draw() {
        return 0;
    }

    let stand_pat = evaluate(pos);

    if stand_pat >= beta {
        return beta;
    }
    if stand_pat > alpha {
        alpha = stand_pat;
    }

    // Generate only captures and promotions.
    let mut moves = Vec::new();
    generate_captures(pos, &mut moves);

    // Order captures by MVV-LVA.
    // We reuse the full ordering via pick-best incremental to allow early cutoffs.
    // For simplicity in quiescence (usually <10 moves), sorting once is fine.
    if moves.is_empty() {
        return alpha;
    }

    // Score and sort descending — small list, full sort is cheap and simple.
    {
        let mut scored: Vec<(i32, crate::board::Move)> = moves
            .into_iter()
            .map(|mv| (crate::search::order::score_move_simple(pos, mv), mv))
            .collect();
        scored.sort_by_key(|b| std::cmp::Reverse(b.0));
        moves = scored.into_iter().map(|(_, mv)| mv).collect();
    }

    // PV handling in quiescence is optional; we maintain length for seldepth
    // but don't store full PV (wastes time). Keep it simple: no PV update.
    // Root PV comes from main search's PV table; qsearch nodes just improve
    // the score that propagates to the PV node.
    // However, to keep PV table consistent, clear this ply's PV length.
    ctx.pv_len[ply] = 0;

    for mv in moves {
        // Make move and check stop quickly.
        if ctx.stop.load(Ordering::Relaxed) {
            return 0;
        }

        pos.make_move(mv);
        let score = -quiescence(pos, -beta, -alpha, ply + 1, ctx);
        pos.unmake_move(mv);

        if ctx.stop.load(Ordering::Relaxed) {
            return 0;
        }

        if score >= beta {
            return beta;
        }
        if score > alpha {
            alpha = score;
        }
    }

    alpha
}

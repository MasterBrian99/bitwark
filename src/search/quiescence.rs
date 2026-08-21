//! Quiescence search — extend search until the position is quiet.
//!
//! Only captures and promotions are searched; quiet moves are ignored.
//! Stand-pat allows the side to move to "pass" if already good enough.
//! See chessprogramming.org "Quiescence Search".

#![allow(clippy::collapsible_if)]

use std::sync::atomic::Ordering;

use crate::board::movelist::MoveList;
use crate::board::{Position, generate_captures_into};
use crate::eval::evaluate_cached;

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
    if ctx.stop.load(Ordering::Relaxed) {
        return 0;
    }
    if ctx.nodes.is_multiple_of(2048) {
        ctx.flush_nodes();
        if ctx.tc.should_hard_stop() {
            ctx.stop.store(true, Ordering::Relaxed);
            return 0;
        }
        if let Some(limit) = ctx.limits.nodes {
            if ctx.total_nodes.load(Ordering::Relaxed) >= limit {
                ctx.stop.store(true, Ordering::Relaxed);
                return 0;
            }
        }
    }

    if ply >= MAX_PLY - 1 {
        return evaluate_cached(pos, &mut ctx.pawn_cache);
    }

    ctx.nodes += 1;
    if ply > ctx.seldepth {
        ctx.seldepth = ply;
    }
    ctx.pv_len[ply] = 0;

    if pos.is_repetition() || pos.is_fifty_move_draw() {
        return 0;
    }

    let stand_pat = evaluate_cached(pos, &mut ctx.pawn_cache);

    if stand_pat >= beta {
        return beta;
    }
    if stand_pat > alpha {
        alpha = stand_pat;
    }

    let mut list = MoveList::new();
    generate_captures_into(pos, &mut list);

    if list.is_empty() {
        return alpha;
    }

    // Score captures by MVV-LVA (history/killers irrelevant in qsearch)
    let dummy_killers = [None, None];
    let dummy_history = [[[0; 64]; 64]; 2];
    crate::search::order::score_list(pos, &mut list, None, &dummy_killers, &dummy_history, ply);

    ctx.pv_len[ply] = 0;

    let len = list.len;
    for i in 0..len {
        if ctx.stop.load(Ordering::Relaxed) {
            return 0;
        }

        let mv = list.pick_best(i);

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

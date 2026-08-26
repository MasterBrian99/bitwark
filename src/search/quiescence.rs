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

use super::{MAX_PLY, SearchContext, tt::Bound};

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
    if ctx.tick_check() {
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

    // TT probe — qsearch entries are stored at depth 0, any hit with
    // sufficient depth can cut off. Mirrors alpha_beta's probe logic.
    let orig_alpha = alpha;
    let tt_hit = ctx.tt.probe(pos.hash(), ply);
    if let Some(hit) = tt_hit {
        match hit.bound {
            Bound::Exact => return hit.score,
            Bound::Lower if hit.score >= beta => return hit.score,
            Bound::Upper if hit.score <= alpha => return hit.score,
            _ => {}
        }
    }

    let stand_pat = evaluate_cached(pos, &mut ctx.pawn_cache);

    if stand_pat >= beta {
        // Store TT entry for stand-pat cutoff
        ctx.tt
            .store(pos.hash(), None, beta, stand_pat, 0, Bound::Lower, ply);
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

    // Score captures — now SEE-aware via order.rs (winning captures above killers)
    // Reuse the earlier TT hit (single probe).
    let tt_move = tt_hit.and_then(|h| h.mv);
    let dummy_killers = [None, None];
    let dummy_history = [[[0; 64]; 64]; 2];
    crate::search::order::score_list(
        pos,
        &mut list,
        tt_move,
        &dummy_killers,
        None,
        &dummy_history,
        None,
        &crate::search::order::ZERO_CONT,
        None,
        &crate::search::order::ZERO_CONT,
        ply,
    );

    ctx.pv_len[ply] = 0;

    let mut best_score = alpha;
    let mut best_move: Option<crate::board::Move> = None;

    let len = list.len;
    for i in 0..len {
        if ctx.stop.load(Ordering::Relaxed) {
            return 0;
        }

        let mv = list.pick_best(i);

        // Delta pruning: if stand_pat + captured piece value + margin < alpha, skip.
        // Promotions are never pruned (they gain a queen).
        if mv.promotion.is_none() {
            let cap_val_opt = if let Some(p) = pos.piece_at(mv.to) {
                Some(match p.piece_type() {
                    crate::board::PieceType::Pawn => 100,
                    crate::board::PieceType::Knight => 320,
                    crate::board::PieceType::Bishop => 330,
                    crate::board::PieceType::Rook => 500,
                    crate::board::PieceType::Queen => 900,
                    crate::board::PieceType::King => 20000,
                })
            } else if pos
                .piece_at(mv.from)
                .map(|p| p.piece_type() == crate::board::PieceType::Pawn)
                .unwrap_or(false)
                && pos.en_passant() == Some(mv.to)
            {
                Some(100)
            } else {
                None
            };
            if let Some(cap_val) = cap_val_opt {
                if stand_pat + cap_val + 200 < alpha {
                    continue;
                }
            }
        }

        // SEE pruning: skip losing captures (SEE < 0), except promotions.
        if mv.promotion.is_none() && crate::search::see::see(pos, mv) < 0 {
            continue;
        }

        pos.make_move(mv);
        let score = -quiescence(pos, -beta, -alpha, ply + 1, ctx);
        pos.unmake_move(mv);

        if ctx.stop.load(Ordering::Relaxed) {
            return 0;
        }

        if score > best_score {
            best_score = score;
            best_move = Some(mv);
        }
        if score >= beta {
            ctx.tt
                .store(pos.hash(), Some(mv), score, stand_pat, 0, Bound::Lower, ply);
            return beta;
        }
        if score > alpha {
            alpha = score;
        }
    }

    // TT store for qsearch result
    let bound = if best_move.is_some() && alpha > orig_alpha {
        Bound::Exact
    } else {
        Bound::Upper
    };
    ctx.tt
        .store(pos.hash(), best_move, alpha, stand_pat, 0, bound, ply);

    alpha
}

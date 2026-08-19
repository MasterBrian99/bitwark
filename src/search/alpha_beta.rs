//! Negamax alpha-beta with PV table, TT, pruning, and extensions.
//!
//! Null-move pruning, late-move reductions with PVS re-search,
//! futility / late-move pruning, and check extensions, plus aspiration at
//! the iterative-deepening level. See chessprogramming.org
//! "Alpha-Beta", "Transposition Table", "Null Move Pruning",
//! "Late Move Reductions", "Futility Pruning", "Check Extensions".

#![allow(clippy::needless_range_loop)]
#![allow(clippy::collapsible_if)]

use std::sync::OnceLock;
use std::sync::atomic::Ordering;

use crate::board::{PieceType, Position, generate_legal, is_square_attacked};

use super::{MATE, MAX_PLY, SearchContext, quiescence::quiescence, tt::Bound};

// ---------------------------------------------------------------------------
// LMR table
// ---------------------------------------------------------------------------

static LMR_TABLE: OnceLock<[[u8; 64]; 64]> = OnceLock::new();

fn lmr_reduction(depth: i32, move_idx: usize) -> i32 {
    let table = LMR_TABLE.get_or_init(|| {
        let mut t = [[0u8; 64]; 64];
        for d in 0..64 {
            for m in 0..64 {
                if d < 3 || m < 3 {
                    t[d][m] = 0;
                } else {
                    let d_f = d as f64;
                    let m_f = m as f64;
                    // 0.77 + ln(d) * ln(m) / 2.36 — classic Tuned
                    let r = 0.77 + d_f.ln() * m_f.ln() / 2.36;
                    t[d][m] = (r as u8).min((d - 2) as u8);
                }
            }
        }
        t
    });
    let d = (depth as usize).min(63);
    let m = move_idx.min(63);
    table[d][m] as i32
}

#[inline]
#[allow(clippy::collapsible_if)]
fn is_quiet(pos: &Position, mv: crate::board::Move) -> bool {
    if mv.promotion.is_some() {
        return false;
    }
    if pos.piece_at(mv.to).is_some() {
        return false;
    }
    // En passant capture
    if let Some(pc) = pos.piece_at(mv.from) {
        if pc.piece_type() == PieceType::Pawn && pos.en_passant() == Some(mv.to) {
            return false;
        }
    }
    true
}

#[inline]
fn gives_check(pos: &mut Position, mv: crate::board::Move) -> bool {
    // Make the move, test if opponent king is attacked, unmake.
    // This is called only for moves considered for LMR (quiet, beyond idx 3),
    // so the extra make/unmake cost is tiny.
    pos.make_move(mv);
    let opp = pos.side_to_move();
    let gives = if let Some(king_sq) = pos.king_square(opp) {
        is_square_attacked(pos, king_sq, opp.opposite())
    } else {
        false
    };
    pos.unmake_move(mv);
    gives
}

/// Negamax alpha-beta.
///
/// `ply` is distance from root, `can_null` allows null-move pruning.
/// Updates `ctx.pv_table`/`pv_len` on PV nodes.
pub fn negamax(
    pos: &mut Position,
    mut depth: i32,
    mut alpha: i32,
    beta: i32,
    ply: usize,
    can_null: bool,
    ctx: &mut SearchContext,
) -> i32 {
    let is_pv = beta > alpha + 1;

    // Abort if stop flag set.
    if ctx.stop.load(Ordering::Relaxed) {
        return 0;
    }
    if ctx.tc.should_hard_stop() {
        ctx.stop.store(true, Ordering::Relaxed);
        return 0;
    }
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
        return crate::eval::evaluate(pos);
    }

    ctx.nodes += 1;
    if ply > ctx.seldepth {
        ctx.seldepth = ply;
    }

    // Draw by 50-move or repetition — searched as 0 (no contempt).
    if pos.is_repetition() || pos.is_fifty_move_draw() {
        return 0;
    }

    // Mate distance pruning (cheap, safe).
    // If we are already mated in ply, alpha/beta bound the mate score.
    let mated = -MATE + ply as i32;
    let mate = MATE - ply as i32 - 1;
    if alpha < mated {
        alpha = mated;
    }
    if beta > mate {
        // beta is exclusive upper bound, but capping is safe.
    }
    if alpha >= beta {
        return alpha;
    }

    // TT probe.
    let mut tt_move: Option<crate::board::Move> = None;
    if let Some(hit) = ctx.tt.probe(pos.hash(), ply) {
        tt_move = hit.mv;
        if (hit.depth as i32) >= depth {
            match hit.bound {
                Bound::Exact => return hit.score,
                Bound::Lower if hit.score >= beta => return hit.score,
                Bound::Upper if hit.score <= alpha => return hit.score,
                _ => {}
            }
        }
    }

    // In-check extension (before leaf check so checks are never qsearched).
    let in_check = if let Some(king_sq) = pos.king_square(pos.side_to_move()) {
        is_square_attacked(pos, king_sq, pos.side_to_move().opposite())
    } else {
        false
    };
    if in_check {
        depth += 1;
    }

    // Leaf: go to quiescence (only when not in check — checks were extended).
    if depth <= 0 {
        return quiescence(pos, alpha, beta, ply, ctx);
    }

    let static_eval = crate::eval::evaluate(pos);

    // Futility pruning / Razoring style: at very low depth and not in check,
    // if eval is far below alpha, return eval (fail-soft). We implement this
    // as per-move quiet skipping below, which is safer for tactics. The node-level
    // razoring shortcut is optional and skipped to keep mate puzzles intact.
    // Null-move pruning (non-PV, not in check, depth >= 3, eval >= beta).
    if !is_pv
        && !in_check
        && can_null
        && depth >= 3
        && static_eval >= beta
        && pos.has_non_pawn_material(pos.side_to_move())
    {
        let r = 2 + depth / 4;
        pos.make_null_move();
        let score = -negamax(pos, depth - 1 - r, -beta, -beta + 1, ply + 1, false, ctx);
        pos.unmake_null_move();
        if ctx.stop.load(Ordering::Relaxed) {
            return 0;
        }
        // Don't return unproven mate scores.
        if score >= beta && score.abs() < MATE - MAX_PLY as i32 {
            return beta;
        }
    }

    // Generate legal moves.
    let mut moves = Vec::new();
    generate_legal(pos, &mut moves);

    if moves.is_empty() {
        if in_check {
            return -MATE + ply as i32;
        }
        return 0;
    }

    ctx.pv_len[ply] = 0;

    let mut best_score = -MATE * 2;
    let original_alpha = alpha;
    let mut best_move: Option<crate::board::Move> = None;
    let mut quiet_tried = 0usize;

    for i in 0..moves.len() {
        // Pick best remaining move.
        let best_idx = {
            let mut best = i;
            let mut best_s = crate::search::order::score_move(
                pos,
                moves[i],
                tt_move,
                &ctx.killers[ply],
                &ctx.history,
                ply,
            );
            for j in (i + 1)..moves.len() {
                let s = crate::search::order::score_move(
                    pos,
                    moves[j],
                    tt_move,
                    &ctx.killers[ply],
                    &ctx.history,
                    ply,
                );
                if s > best_s {
                    best_s = s;
                    best = j;
                }
            }
            best
        };
        moves.swap(i, best_idx);
        let mv = moves[i];

        let quiet = is_quiet(pos, mv);

        // Futility & LMP skips for quiet moves (captures always searched).
        if !is_pv && !in_check && quiet {
            // Futility: skip quiets that are very unlikely to raise alpha.
            if depth <= 6 && static_eval + 120 * depth <= alpha {
                continue;
            }
            // Late move pruning: skip late quiets at shallow depth.
            if depth <= 4 && quiet_tried >= 3 + (depth as usize * depth as usize) {
                continue;
            }
        }
        if quiet {
            quiet_tried += 1;
        }

        if ctx.stop.load(Ordering::Relaxed) {
            return 0;
        }

        // LMR: reduce quiet non-check-giving moves beyond the first few.
        let mut reduction = 0;
        if !is_pv && !in_check && quiet && depth >= 3 && i >= 3 && !gives_check(pos, mv) {
            reduction = lmr_reduction(depth, i);
            reduction = reduction.clamp(1, depth - 2);
            if reduction < 0 {
                reduction = 0;
            }
        }

        let score = if reduction > 0 {
            // Reduced zero-window search.
            pos.make_move(mv);
            let v = -negamax(
                pos,
                depth - 1 - reduction,
                -alpha - 1,
                -alpha,
                ply + 1,
                true,
                ctx,
            );
            pos.unmake_move(mv);
            if ctx.stop.load(Ordering::Relaxed) {
                return 0;
            }
            if v > alpha {
                // Re-search at full depth, zero-window.
                pos.make_move(mv);
                let v2 = -negamax(pos, depth - 1, -alpha - 1, -alpha, ply + 1, true, ctx);
                pos.unmake_move(mv);
                if ctx.stop.load(Ordering::Relaxed) {
                    return 0;
                }
                if v2 > alpha && is_pv {
                    pos.make_move(mv);
                    let v3 = -negamax(pos, depth - 1, -beta, -alpha, ply + 1, true, ctx);
                    pos.unmake_move(mv);
                    if ctx.stop.load(Ordering::Relaxed) {
                        return 0;
                    }
                    v3
                } else {
                    v2
                }
            } else {
                v
            }
        } else if i == 0 {
            pos.make_move(mv);
            let v = -negamax(pos, depth - 1, -beta, -alpha, ply + 1, true, ctx);
            pos.unmake_move(mv);
            if ctx.stop.load(Ordering::Relaxed) {
                return 0;
            }
            v
        } else {
            // PVS zero-window.
            pos.make_move(mv);
            let v = -negamax(pos, depth - 1, -alpha - 1, -alpha, ply + 1, true, ctx);
            pos.unmake_move(mv);
            if ctx.stop.load(Ordering::Relaxed) {
                return 0;
            }
            if v > alpha {
                pos.make_move(mv);
                let v2 = -negamax(pos, depth - 1, -beta, -alpha, ply + 1, true, ctx);
                pos.unmake_move(mv);
                if ctx.stop.load(Ordering::Relaxed) {
                    return 0;
                }
                v2
            } else {
                v
            }
        };

        if score > best_score {
            best_score = score;
            best_move = Some(mv);
        }

        if score > alpha {
            alpha = score;
            ctx.pv_table[ply][0] = Some(mv);
            let child_len = ctx.pv_len[ply + 1];
            for j in 0..child_len {
                ctx.pv_table[ply][j + 1] = ctx.pv_table[ply + 1][j];
            }
            ctx.pv_len[ply] = child_len + 1;

            if alpha >= beta {
                // Beta cutoff: update killers/history if quiet.
                if quiet {
                    crate::search::order::update_killers(&mut ctx.killers, ply, mv);
                    let side = pos.side_to_move();
                    crate::search::order::update_history(&mut ctx.history, side, mv, depth);
                }
                break;
            }
        }
    }

    let bound = if best_score <= original_alpha {
        Bound::Upper
    } else if best_score >= beta {
        Bound::Lower
    } else {
        Bound::Exact
    };
    let store_depth = depth.clamp(0, 127) as u8;
    ctx.tt.store(
        pos.hash(),
        best_move,
        best_score,
        static_eval,
        store_depth,
        bound,
        ply,
    );

    if best_score > original_alpha {
        alpha
    } else {
        best_score
    }
}

//! Negamax alpha-beta with PV table and draw detection.
//!
//! Phase 3 has no pruning, null move, LMR, etc. — that is Phase 4.
//! The only extensions are mate-distance handling.
//! See chessprogramming.org "Alpha-Beta" and "Principal Variation".

use std::sync::atomic::Ordering;

use crate::board::{Position, generate_legal, is_square_attacked};

use super::{MATE, MAX_PLY, SearchContext, quiescence::quiescence};

/// Negamax alpha-beta.
///
/// Returns the score from the side-to-move's perspective.  `ply` is the
/// distance from the root (0 at root).  Updates `ctx.pv_table`/`pv_len` on
/// PV nodes.  Fail-soft: may return outside [alpha, beta] on cutoffs.
pub fn negamax(
    pos: &mut Position,
    depth: i32,
    mut alpha: i32,
    beta: i32,
    ply: usize,
    ctx: &mut SearchContext,
) -> i32 {
    // Abort if stop flag set.
    if ctx.stop.load(Ordering::Relaxed) {
        return 0;
    }
    // Periodic time check (every 2048 nodes).
    if ctx.nodes.is_multiple_of(2048) && ctx.should_stop_time() {
        ctx.stop.store(true, Ordering::Relaxed);
        return 0;
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

    // Leaf: go to quiescence.
    if depth <= 0 {
        return quiescence(pos, alpha, beta, ply, ctx);
    }

    // Generate legal moves.
    let mut moves = Vec::new();
    generate_legal(pos, &mut moves);

    // Mate / stalemate detection.
    if moves.is_empty() {
        // No legal moves: check if in check.
        if let Some(king_sq) = pos.king_square(pos.side_to_move())
            && is_square_attacked(pos, king_sq, pos.side_to_move().opposite())
        {
            // Checkmate: prefer mates closer to root (smaller ply).
            return -MATE + ply as i32;
        }
        // Stalemate (or missing king) → draw.
        return 0;
    }

    // Initialize PV for this ply.
    ctx.pv_len[ply] = 0;

    let mut best_score = -MATE * 2; // -infinity
    let original_alpha = alpha;

    // Incremental pick-best: find best remaining move each iteration.
    // This avoids sorting the tail that will be pruned by beta cutoffs.
    #[allow(clippy::needless_range_loop)]
    for i in 0..moves.len() {
        // Pick best move among moves[i..]
        let best_idx = {
            let mut best = i;
            let mut best_s = crate::search::order::score_move(pos, moves[i]);
            for j in (i + 1)..moves.len() {
                let s = crate::search::order::score_move(pos, moves[j]);
                if s > best_s {
                    best_s = s;
                    best = j;
                }
            }
            best
        };
        moves.swap(i, best_idx);
        let mv = moves[i];

        if ctx.stop.load(Ordering::Relaxed) {
            return 0;
        }

        pos.make_move(mv);
        let score = -negamax(pos, depth - 1, -beta, -alpha, ply + 1, ctx);
        pos.unmake_move(mv);

        if ctx.stop.load(Ordering::Relaxed) {
            return 0;
        }

        if score > best_score {
            best_score = score;
        }

        if score > alpha {
            alpha = score;
            // Update PV: this move followed by child's PV.
            ctx.pv_table[ply][0] = Some(mv);
            let child_len = ctx.pv_len[ply + 1];
            for j in 0..child_len {
                ctx.pv_table[ply][j + 1] = ctx.pv_table[ply + 1][j];
            }
            ctx.pv_len[ply] = child_len + 1;

            if alpha >= beta {
                break; // beta cutoff (fail-soft, return best_score)
            }
        }
    }

    // If we never raised alpha, best_score is the best among all moves
    // (fail-low) and PV remains empty for this ply (length 0).  Return it
    // so the caller sees a score ≤ original_alpha.
    if best_score > original_alpha {
        // We raised alpha somewhere; best_score == alpha.
        alpha
    } else {
        best_score
    }
}

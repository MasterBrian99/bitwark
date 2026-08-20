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

use crate::board::movelist::MoveList;
use crate::board::types::Bitboard;
use crate::board::{
    Move, Piece, PieceType, Position, Square, generate_legal_into, is_square_attacked,
};
use crate::board::{bishop_attacks, knight_attacks, pawn_attacks, rook_attacks};

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
fn is_quiet(pos: &Position, mv: Move) -> bool {
    if mv.promotion.is_some() {
        return false;
    }
    if pos.piece_at(mv.to).is_some() {
        return false;
    }
    if let Some(pc) = pos.piece_at(mv.from) {
        if pc.piece_type() == PieceType::Pawn && pos.en_passant() == Some(mv.to) {
            return false;
        }
    }
    true
}

#[inline]
fn gives_check(pos: &Position, mv: Move) -> bool {
    // Bitboard gives-check without make/unmake: does the move attack the enemy king
    // with its post-move occupancy? Covers direct and discovered.
    let us = pos.side_to_move();
    let them = us.opposite();
    let ek = match pos.king_square(them) {
        Some(s) => s,
        None => return false,
    };
    let moving = match pos.piece_at(mv.from) {
        Some(p) => p,
        None => return false,
    };
    let occ = pos.occupied();
    let from_bb = Bitboard::from_sq(mv.from);
    let to_bb = Bitboard::from_sq(mv.to);
    let is_ep = moving.piece_type() == PieceType::Pawn && pos.en_passant() == Some(mv.to);
    let occ_after = if is_ep {
        let cap_sq = Square::from_coords(mv.to.file(), mv.from.rank());
        let cap_bb = Bitboard::from_sq(cap_sq);
        Bitboard((occ.0 ^ from_bb.0 ^ cap_bb.0) | to_bb.0)
    } else {
        Bitboard((occ.0 ^ from_bb.0) | to_bb.0)
    };

    // After bitboards for us attackers
    let mut us_pawns = pos.pieces_bb(Piece::new(us, PieceType::Pawn));
    let mut us_knights = pos.pieces_bb(Piece::new(us, PieceType::Knight));
    let mut us_bishops = pos.pieces_bb(Piece::new(us, PieceType::Bishop));
    let mut us_rooks = pos.pieces_bb(Piece::new(us, PieceType::Rook));
    let mut us_queens = pos.pieces_bb(Piece::new(us, PieceType::Queen));

    match moving.piece_type() {
        PieceType::Pawn => {
            us_pawns.0 &= !from_bb.0;
            if let Some(promo) = mv.promotion {
                match promo {
                    PieceType::Queen => us_queens.0 |= to_bb.0,
                    PieceType::Rook => us_rooks.0 |= to_bb.0,
                    PieceType::Bishop => us_bishops.0 |= to_bb.0,
                    PieceType::Knight => us_knights.0 |= to_bb.0,
                    PieceType::Pawn | PieceType::King => {}
                }
            } else {
                us_pawns.0 |= to_bb.0;
            }
        }
        PieceType::Knight => {
            us_knights.0 ^= from_bb.0 | to_bb.0;
        }
        PieceType::Bishop => {
            us_bishops.0 ^= from_bb.0 | to_bb.0;
        }
        PieceType::Rook => {
            us_rooks.0 ^= from_bb.0 | to_bb.0;
        }
        PieceType::Queen => {
            us_queens.0 ^= from_bb.0 | to_bb.0;
        }
        PieceType::King => {}
    }

    // Pawns attack
    if (pawn_attacks(ek, them) & us_pawns).0 != 0 {
        return true;
    }
    if (knight_attacks(ek) & us_knights).0 != 0 {
        return true;
    }
    // King cannot give check by moving adjacent (illegal), but discovered via other piece still counts — handled by sliders below.
    let bishops = us_bishops | us_queens;
    if (bishop_attacks(ek, occ_after) & bishops).0 != 0 {
        return true;
    }
    let rooks = us_rooks | us_queens;
    if (rook_attacks(ek, occ_after) & rooks).0 != 0 {
        return true;
    }
    false
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

    if ctx.stop.load(Ordering::Relaxed) {
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

    if pos.is_repetition() || pos.is_fifty_move_draw() {
        return 0;
    }

    let mated = -MATE + ply as i32;
    // let mate = MATE - ply as i32 - 1; // beta capping is safe but not needed (exclusive bound)
    if alpha < mated {
        alpha = mated;
    }
    if alpha >= beta {
        return alpha;
    }

    let mut tt_move: Option<Move> = None;
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

    let in_check = if let Some(king_sq) = pos.king_square(pos.side_to_move()) {
        is_square_attacked(pos, king_sq, pos.side_to_move().opposite())
    } else {
        false
    };
    if in_check {
        depth += 1;
    }

    if depth <= 0 {
        return quiescence(pos, alpha, beta, ply, ctx);
    }

    let static_eval = crate::eval::evaluate(pos);

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
        if score >= beta && score.abs() < MATE - MAX_PLY as i32 {
            return beta;
        }
    }

    // Generate legal moves into stack MoveList
    let mut list = MoveList::new();
    generate_legal_into(pos, &mut list);

    if list.is_empty() {
        if in_check {
            return -MATE + ply as i32;
        }
        return 0;
    }

    // Pre-score once (O(n) instead of O(n²))
    crate::search::order::score_list(
        pos,
        &mut list,
        tt_move,
        &ctx.killers[ply],
        &ctx.history,
        ply,
    );

    ctx.pv_len[ply] = 0;

    let mut best_score = -MATE * 2;
    let original_alpha = alpha;
    let mut best_move: Option<Move> = None;
    let mut quiet_tried = 0usize;

    let list_len = list.len;
    for i in 0..list_len {
        let mv = list.pick_best(i);

        let quiet = is_quiet(pos, mv);

        if !is_pv && !in_check && quiet {
            if depth <= 6 && static_eval + 120 * depth <= alpha {
                continue;
            }
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

        let mut reduction = 0;
        if !is_pv && !in_check && quiet && depth >= 3 && i >= 3 && !gives_check(pos, mv) {
            reduction = lmr_reduction(depth, i);
            reduction = reduction.clamp(1, depth - 2);
            if reduction < 0 {
                reduction = 0;
            }
        }

        let score = if reduction > 0 {
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

#[cfg(test)]
mod gives_check_tests {
    use super::gives_check;
    use crate::board::{Move, Position, generate_legal, is_square_attacked};

    fn gives_check_slow(pos: &mut Position, mv: Move) -> bool {
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

    #[test]
    fn gives_check_matches_random() {
        use crate::board::fen::parse_fen;
        let fens = [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
            "r1bq1rk1/pp2ppbp/2np1np1/2p5/2P1P3/1PN1B3/PB1Q1PPP/R3K2R w KQ - 0 1",
        ];
        for fen in fens {
            let pos = parse_fen(fen).unwrap();
            let mut moves = Vec::new();
            generate_legal(&pos, &mut moves);
            for mv in moves {
                let fast = gives_check(&pos, mv);
                let mut pos2 = pos.clone();
                let slow = gives_check_slow(&mut pos2, mv);
                assert_eq!(
                    fast, slow,
                    "mismatch fen {} mv {} fast {} slow {}",
                    fen, mv, fast, slow
                );
            }
        }
        // random walk
        let mut pos =
            parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let mut rng: u64 = 0x9E3779B97F4A7C15;
        for _ in 0..200 {
            let mut moves = Vec::new();
            generate_legal(&pos, &mut moves);
            if moves.is_empty() {
                break;
            }
            for mv in &moves {
                let fast = gives_check(&pos, *mv);
                let mut pos2 = pos.clone();
                let slow = gives_check_slow(&mut pos2, *mv);
                assert_eq!(
                    fast,
                    slow,
                    "walk mismatch pos {} mv {} fast {} slow {}",
                    crate::board::fen::to_fen(&pos),
                    mv,
                    fast,
                    slow
                );
            }
            // pick pseudo random move
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let idx = (rng as usize) % moves.len();
            pos.make_move(moves[idx]);
            // small chance to undo? keep walk.
        }
    }
}

//! Move ordering — MVV-LVA for captures and promotions.
//!
//! Phase 3 uses a minimal scheme: captures and promotions sorted by
//! MVV-LVA, quiets all 0.  TT moves / killers / history arrive in Phase 4.
//!
//! Scoring: `10 * victim_value - attacker_value + promo_bonus`.
//! Higher is searched first.  See chessprogramming.org "MVV-LVA".

use crate::board::{Move, PieceType, Position};

// Piece values for MVV-LVA (centipawns, same as eval material but King = 20000).
fn piece_value(pt: PieceType) -> i32 {
    match pt {
        PieceType::Pawn => 100,
        PieceType::Knight => 320,
        PieceType::Bishop => 330,
        PieceType::Rook => 500,
        PieceType::Queen => 900,
        PieceType::King => 20000,
    }
}

/// Score a move for ordering.  `pos` is the position *before* the move.
pub fn score_move(pos: &Position, mv: Move) -> i32 {
    let moving = match pos.piece_at(mv.from) {
        Some(p) => p,
        None => return 0,
    };
    let attacker_val = piece_value(moving.piece_type());

    // Captured piece, if any (including en passant).
    let captured = if let Some(p) = pos.piece_at(mv.to) {
        Some(p)
    } else if moving.piece_type() == PieceType::Pawn && pos.en_passant() == Some(mv.to) {
        // En passant captures a pawn on the from-rank.
        // The captured pawn is always a pawn, value 100.
        // We treat it as victim = pawn.
        // Return MVV score directly.
        let victim_val = 100;
        let promo_bonus = if let Some(pt) = mv.promotion {
            piece_value(pt)
        } else {
            0
        };
        return 10 * victim_val - attacker_val + promo_bonus;
    } else {
        None
    };

    if let Some(cap) = captured {
        let victim_val = piece_value(cap.piece_type());
        let promo_bonus = if let Some(pt) = mv.promotion {
            // Promotion capture is extra valuable — add promo piece value.
            piece_value(pt)
        } else {
            0
        };
        return 10 * victim_val - attacker_val + promo_bonus;
    }

    // Non-capture promotion (quiet promo) — still very tactical.
    if let Some(pt) = mv.promotion {
        // Queen promo push is almost as good as a capture.
        return 800 + piece_value(pt);
    }

    // Quiet move.
    0
}

/// Find the index of the highest-scored move in `moves[pos..]` according to
/// `score_move`, from `pos` before the move.  Returns 0-based index within
/// the slice (so caller adds the offset).
#[allow(dead_code)]
pub fn pick_best(moves: &[Move], pos: &Position) -> usize {
    debug_assert!(!moves.is_empty());
    let mut best_idx = 0;
    let mut best_score = score_move(pos, moves[0]);
    for (i, &mv) in moves.iter().enumerate().skip(1) {
        let s = score_move(pos, mv);
        if s > best_score {
            best_score = s;
            best_idx = i;
        }
    }
    best_idx
}

/// Sort `moves` in-place descending by MVV-LVA.  Used when we want a full
/// sorted list (e.g. for debugging).  The search hot path uses `pick_best`
/// incrementally to avoid sorting the tail that will be pruned.
#[allow(dead_code)]
pub fn sort_moves(moves: &mut [Move], pos: &Position) {
    moves.sort_by_key(|&mv| std::cmp::Reverse(score_move(pos, mv)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::parse_fen;

    #[test]
    fn mvv_lva_capture_order() {
        // Position with multiple captures: queen takes rook should beat pawn takes pawn, etc.
        let pos = parse_fen("4k3/3q4/8/3r4/4P3/8/8/4K3 w - - 0 1").unwrap();
        // White pawn on e4 can capture? Not relevant; construct moves manually
        // Score pawn captures queen vs pawn captures rook.
        let q_cap = crate::board::mv::Move::parse_uci("e4d5").unwrap(); // pawn takes pawn? Let's make clearer
        // Instead craft a simple board: white queen on d1, black rook on d8, black pawn on e5
        let pos2 = parse_fen("3rk3/8/8/4p3/8/8/8/3QK3 w - - 0 1").unwrap();
        let mv_qxr = crate::board::mv::Move::parse_uci("d1d8").unwrap(); // Q x R
        let mv_qxp = crate::board::mv::Move::parse_uci("d1e2").unwrap(); // quiet
        // QxR should score higher than quiet
        assert!(score_move(&pos2, mv_qxr) > score_move(&pos2, mv_qxp));
        // Rook value 500 vs pawn 100, so QxR (victim 500) < Qx? Actually queen takes rook vs pawn
        // Pawn victim smaller, so score lower.
        let mv_qxp2 = {
            // Queen takes pawn on e5 if we place queen near
            // Use custom pos: white queen d4, black pawn e5
            let p = parse_fen("4k3/8/8/4p3/3Q4/8/8/4K3 w - - 0 1").unwrap();
            let mv = crate::board::mv::Move::parse_uci("d4e5").unwrap();
            // capture pawn
            score_move(&p, mv)
        };
        let score_rook_cap = score_move(&pos2, mv_qxr);
        assert!(score_rook_cap > mv_qxp2);
    }

    #[test]
    fn promo_scores_high() {
        let pos = parse_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let promo_q = crate::board::mv::Move::parse_uci("a7a8q").unwrap();
        let quiet = crate::board::mv::Move::parse_uci("a7a8q").unwrap(); // same
        // Quiet promo should have high score >0
        assert!(score_move(&pos, promo_q) > 0);
    }
}

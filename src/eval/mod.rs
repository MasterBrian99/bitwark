//! Evaluation v1 — material + piece-square tables.
//!
//! Phase 3 uses the classic "Simplified Evaluation Function" by Tomasz
//! Michniewski (chessprogramming.org "Simplified Evaluation Function").
//! One set of tables, no tapering — the midgame king table is used for
//! all phases.  Tapering (MG/EG interpolation) arrives in Phase 5.
//!
//! Scores are centipawns from the side-to-move's perspective (negamax
//! convention): positive = good for the player who must move.
//!
//! ```text
//! evaluate(pos) = (white_material + white_PST) - (black_material + black_PST)
//!                 then negated if black to move
//! ```
//!
//! PSTs are stored with `a1 = 0` (`Square::index`).  Black pieces mirror
//! vertically via `sq ^ 56` (flipping rank bits).

use crate::board::{Color, Position, Square};

// ---------------------------------------------------------------------------
// Material values (centipawns)
// ---------------------------------------------------------------------------

const MATERIAL: [i32; 6] = [
    100, // Pawn
    320, // Knight
    330, // Bishop
    500, // Rook
    900, // Queen
    0,   // King (value folded into PST)
];

// ---------------------------------------------------------------------------
// Piece-square tables — Michniewski simplified, white perspective, a1 = 0
// ---------------------------------------------------------------------------
//
// Layout: index 0 = a1, 7 = h1, 8 = a2, ... 63 = h8.
// Each table below is the displayed table (rank 8 at top) with rows reversed
// so that rank 1 occupies indices 0..7.

const PST_PAWN: [i32; 64] = [
    0, 0, 0, 0, 0, 0, 0, 0, // rank 1
    5, 10, 10, -20, -20, 10, 10, 5, // rank 2
    5, -5, -10, 0, 0, -10, -5, 5, // rank 3
    0, 0, 0, 20, 20, 0, 0, 0, // rank 4
    5, 5, 10, 25, 25, 10, 5, 5, // rank 5
    10, 10, 20, 30, 30, 20, 10, 10, // rank 6
    50, 50, 50, 50, 50, 50, 50, 50, // rank 7
    0, 0, 0, 0, 0, 0, 0, 0, // rank 8
];

const PST_KNIGHT: [i32; 64] = [
    -50, -40, -30, -30, -30, -30, -40, -50, // rank 1
    -40, -20, 0, 5, 5, 0, -20, -40, // rank 2
    -30, 5, 10, 15, 15, 10, 5, -30, // rank 3
    -30, 0, 15, 20, 20, 15, 0, -30, // rank 4
    -30, 5, 15, 20, 20, 15, 5, -30, // rank 5
    -30, 0, 10, 15, 15, 10, 0, -30, // rank 6
    -40, -20, 0, 0, 0, 0, -20, -40, // rank 7
    -50, -40, -30, -30, -30, -30, -40, -50, // rank 8
];

const PST_BISHOP: [i32; 64] = [
    -20, -10, -10, -10, -10, -10, -10, -20, // rank 1
    -10, 5, 0, 0, 0, 0, 5, -10, // rank 2
    -10, 10, 10, 10, 10, 10, 10, -10, // rank 3
    -10, 0, 10, 10, 10, 10, 0, -10, // rank 4
    -10, 5, 5, 10, 10, 5, 5, -10, // rank 5
    -10, 0, 5, 10, 10, 5, 0, -10, // rank 6
    -10, 0, 0, 0, 0, 0, 0, -10, // rank 7
    -20, -10, -10, -10, -10, -10, -10, -20, // rank 8
];

const PST_ROOK: [i32; 64] = [
    0, 0, 0, 5, 5, 0, 0, 0, // rank 1
    -5, 0, 0, 0, 0, 0, 0, -5, // rank 2
    -5, 0, 0, 0, 0, 0, 0, -5, // rank 3
    -5, 0, 0, 0, 0, 0, 0, -5, // rank 4
    -5, 0, 0, 0, 0, 0, 0, -5, // rank 5
    -5, 0, 0, 0, 0, 0, 0, -5, // rank 6
    5, 10, 10, 10, 10, 10, 10, 5, // rank 7
    0, 0, 0, 0, 0, 0, 0, 0, // rank 8
];

const PST_QUEEN: [i32; 64] = [
    -20, -10, -10, -5, -5, -10, -10, -20, // rank 1
    -10, 0, 5, 0, 0, 0, 0, -10, // rank 2
    -10, 5, 5, 5, 5, 5, 0, -10, // rank 3
    0, 0, 5, 5, 5, 5, 0, -5, // rank 4
    -5, 0, 5, 5, 5, 5, 0, -5, // rank 5
    -10, 0, 5, 5, 5, 5, 0, -10, // rank 6
    -10, 0, 0, 0, 0, 0, 0, -10, // rank 7
    -20, -10, -10, -5, -5, -10, -10, -20, // rank 8
];

const PST_KING: [i32; 64] = [
    20, 30, 10, 0, 0, 10, 30, 20, // rank 1
    20, 20, 0, 0, 0, 0, 20, 20, // rank 2
    -10, -20, -20, -20, -20, -20, -20, -10, // rank 3
    -20, -30, -30, -40, -40, -30, -30, -20, // rank 4
    -30, -40, -40, -50, -50, -40, -40, -30, // rank 5
    -30, -40, -40, -50, -50, -40, -40, -30, // rank 6
    -30, -40, -40, -50, -50, -40, -40, -30, // rank 7
    -30, -40, -40, -50, -50, -40, -40, -30, // rank 8
];

const PST_TABLES: [[i32; 64]; 6] = [
    PST_PAWN, PST_KNIGHT, PST_BISHOP, PST_ROOK, PST_QUEEN, PST_KING,
];

/// Evaluate `pos` in centipawns from the side-to-move's perspective.
///
/// Positive → good for the player to move.  Uses material + PST only.
/// Missing kings (illegal FENs) are tolerated — evaluation just omits them.
pub fn evaluate(pos: &Position) -> i32 {
    let mut white_score: i32 = 0;
    let mut black_score: i32 = 0;

    for sq_idx in 0u8..64 {
        let sq = Square::new(sq_idx);
        if let Some(piece) = pos.piece_at(sq) {
            let ty = piece.piece_type() as usize;
            let mat = MATERIAL[ty];
            let pst = PST_TABLES[ty][if piece.color() == Color::White {
                sq_idx as usize
            } else {
                (sq_idx ^ 56) as usize
            }];
            let total = mat + pst;
            match piece.color() {
                Color::White => white_score += total,
                Color::Black => black_score += total,
            }
        }
    }

    let score = white_score - black_score;
    if pos.side_to_move() == Color::White {
        score
    } else {
        -score
    }
}

/// Raw material+PST without side-to-move negation — useful for tests.
#[allow(dead_code)]
pub fn evaluate_raw(pos: &Position) -> i32 {
    let mut white_score: i32 = 0;
    let mut black_score: i32 = 0;
    for sq_idx in 0u8..64 {
        let sq = Square::new(sq_idx);
        if let Some(piece) = pos.piece_at(sq) {
            let ty = piece.piece_type() as usize;
            let mat = MATERIAL[ty];
            let pst = PST_TABLES[ty][if piece.color() == Color::White {
                sq_idx as usize
            } else {
                (sq_idx ^ 56) as usize
            }];
            let total = mat + pst;
            match piece.color() {
                Color::White => white_score += total,
                Color::Black => black_score += total,
            }
        }
    }
    white_score - black_score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::parse_fen;

    #[test]
    fn startpos_zero() {
        let pos = crate::board::Position::startpos();
        // Symmetric PSTs → 0 regardless of side to move (white to move)
        assert_eq!(evaluate(&pos), 0);
    }

    #[test]
    fn startpos_black_to_move_zero() {
        // Flip side via FEN with black to move — still symmetric
        let pos2 = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1").unwrap();
        assert_eq!(evaluate(&pos2), 0);
    }

    #[test]
    fn mirror_symmetry() {
        // White pawn on e4 vs black pawn on e5 should be opposite scores when side to move flips.
        // Build two positions mirrored vertically and with colors swapped.
        let fen_white = "4k3/8/8/8/4P3/8/8/4K3 w - - 0 1";
        let fen_black = "4k3/8/8/4p3/8/8/8/4K3 w - - 0 1"; // not mirrored, just pawn color flipped?
        let pos_w = parse_fen(fen_white).unwrap();
        let pos_b = parse_fen(fen_black).unwrap();
        // Raw scores (white perspective) should be opposite
        let raw_w = evaluate_raw(&pos_w);
        let raw_b = evaluate_raw(&pos_b);
        // White pawn on e4 ≈ 100+25, black pawn on e5 same but black side → raw scores opposite sign
        assert_eq!(raw_w, -raw_b);
    }

    #[test]
    fn pawn_mirror_pst() {
        // White pawn on e4 and black pawn on e5 have same PST because black mirrors.
        let pos_white_e4 = parse_fen("4k3/8/8/8/4P3/8/8/4K3 w - - 0 1").unwrap();
        let pos_black_e5 = parse_fen("4k3/8/8/4p3/8/8/8/4K3 w - - 0 1").unwrap();
        // Both pawns are one step beyond center; raw white perspective: one is +ve, other -ve, magnitude equal
        let rw = evaluate_raw(&pos_white_e4);
        let rb = evaluate_raw(&pos_black_e5);
        assert_eq!(rw, -rb);
        assert!(rw > 0);
    }

    #[test]
    fn queen_up_big_advantage() {
        let fen = "4k3/8/8/8/8/8/8/3QK3 w - - 0 1";
        let pos = parse_fen(fen).unwrap();
        let s = evaluate(&pos);
        assert!(s > 800, "queen up should be >800, got {s}");
        // Black to move same position should be negative of white to move (score is side-to-move)
        let fen_b = "4k3/8/8/8/8/8/8/3QK3 b - - 0 1";
        let pos_b = parse_fen(fen_b).unwrap();
        let s_b = evaluate(&pos_b);
        assert_eq!(s, -s_b);
    }

    #[test]
    fn knight_center_vs_rim() {
        // Knight on d4 (center, PST 20) should beat knight on a1 (corner, -50)
        let fen_center = "4k3/8/8/8/3N4/8/8/4K3 w - - 0 1";
        let fen_rim = "4k3/8/8/8/8/8/8/N3K3 w - - 0 1";
        let pos_c = parse_fen(fen_center).unwrap();
        let pos_r = parse_fen(fen_rim).unwrap();
        assert!(
            evaluate(&pos_c) > evaluate(&pos_r),
            "center {} vs rim {}",
            evaluate(&pos_c),
            evaluate(&pos_r)
        );
    }

    #[test]
    fn no_panic_on_kingless() {
        // FEN requires kings, so craft an empty board directly via the
        // crate-private helper — evaluation must not assume kings exist.
        let pos = crate::board::Position::empty_for_fen_internal();
        let _ = evaluate(&pos);
        // Also with a lone pawn and no kings
        let mut pos2 = crate::board::Position::empty_for_fen_internal();
        // Place a white pawn on e4 by reusing the fen helper: parse then strip kings
        // Instead just test that evaluate on a board with only kings doesn't panic
        let fen_kings = "4k3/8/8/8/8/8/8/4K3 w - - 0 1";
        let pos3 = parse_fen(fen_kings).unwrap();
        let _ = evaluate(&pos3);
        let _ = pos2;
    }
}

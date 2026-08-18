//! Pawn structure — isolated, doubled, passed.
//!
//! Why pawn structure matters: pawns define the skeleton of the position.
//! Doubled pawns are immobile and weak, isolated pawns need piece protection,
//! passed pawns are future queens. These are classic handcrafted terms
//! (chessprogramming.org "Pawn Structure" / "Doubled Pawn", "Isolated Pawn",
//! "Passed Pawn"). All bonuses are tapered MG/EG and summed white − black.

use crate::board::{Bitboard, Color, Piece, PieceType, Position, Square};

// Penalties are negative (applied per pawn/file).
const ISOLATED_PENALTY_MG: i32 = -10;
const ISOLATED_PENALTY_EG: i32 = -15;
const DOUBLED_PENALTY_MG: i32 = -11;
const DOUBLED_PENALTY_EG: i32 = -18;

// Passed pawn bonus per rank (index = rank 0..7, rank 1 = promotion impossible = 0).
// White's perspective: rank increases toward promotion (rank 7 large).
// For Black we mirror: bonus_rank = 7 - rank.
const PASSED_BONUS_MG: [i32; 8] = [0, 5, 8, 15, 25, 40, 60, 0];
const PASSED_BONUS_EG: [i32; 8] = [0, 10, 20, 35, 60, 90, 130, 0];

/// Evaluate pawn structure, returning (mg, eg) white − black.
pub fn eval(pos: &Position) -> (i32, i32) {
    let white_pawns = pos.pieces_bb(Piece::new(Color::White, PieceType::Pawn));
    let black_pawns = pos.pieces_bb(Piece::new(Color::Black, PieceType::Pawn));

    let (w_mg, w_eg) = eval_side(pos, Color::White, white_pawns, black_pawns);
    let (b_mg, b_eg) = eval_side(pos, Color::Black, black_pawns, white_pawns);

    (w_mg - b_mg, w_eg - b_eg)
}

fn eval_side(
    _pos: &Position,
    color: Color,
    own_pawns: Bitboard,
    enemy_pawns: Bitboard,
) -> (i32, i32) {
    if own_pawns.is_empty() {
        return (0, 0);
    }

    let mut mg: i32 = 0;
    let mut eg: i32 = 0;

    // Precompute per-file counts for doubled detection.
    let mut file_counts = [0u8; 8];
    for sq in own_pawns.squares() {
        file_counts[sq.file() as usize] += 1;
    }

    for sq in own_pawns.squares() {
        let file = sq.file() as usize;
        let rank = sq.rank() as usize;

        // Doubled: extra pawns beyond the first on the same file.
        // To avoid double-counting, only the "extra" pawns are penalised:
        // we penalise each pawn if count >1, but distribute (count-1) penalties
        // across the file's pawns by applying penalty per pawn and later dividing?
        // Simpler: for each file with n pawns, total penalty = (n-1) * PENALTY.
        // We do per-pawn check: if file_counts[file] > 1, we will later adjust.
        // Instead apply per pawn: if not the first pawn on that file encountered,
        // apply penalty. Use a seen flag per file.
        // For simplicity in this per-pawn loop, apply penalty to every pawn on
        // a doubled file, then outside divide? Instead do file-level aggregation
        // below — skip per-pawn doubled here and handle after loop.
        let _ = rank;

        // Isolated: no friendly pawn on adjacent files.
        let mut adjacent_files_empty = true;
        if file > 0 && file_counts[file - 1] > 0 {
            adjacent_files_empty = false;
        }
        if file < 7 && file_counts[file + 1] > 0 {
            adjacent_files_empty = false;
        }
        if adjacent_files_empty {
            mg += ISOLATED_PENALTY_MG;
            eg += ISOLATED_PENALTY_EG;
        }

        // Passed: no enemy pawn on same or adjacent files ahead.
        if is_passed(sq, color, enemy_pawns) {
            let bonus_rank = if color == Color::White {
                rank
            } else {
                7 - rank
            };
            mg += PASSED_BONUS_MG[bonus_rank];
            eg += PASSED_BONUS_EG[bonus_rank];
        }
    }

    // Doubled: per-file (count-1) penalty.
    for &cnt_u8 in &file_counts {
        let cnt = cnt_u8 as i32;
        if cnt > 1 {
            let extra = cnt - 1;
            mg += extra * DOUBLED_PENALTY_MG;
            eg += extra * DOUBLED_PENALTY_EG;
        }
    }

    (mg, eg)
}

fn is_passed(sq: Square, color: Color, enemy_pawns: Bitboard) -> bool {
    let file = sq.file() as i32;
    let rank = sq.rank() as i32;

    // Enemy pawns that could block: same file and adjacent files, ahead.
    for df in -1..=1 {
        let f = file + df;
        if !(0..8).contains(&f) {
            continue;
        }
        let file_bb = Bitboard::file_bb(f as u8);
        let blockers = enemy_pawns.0 & file_bb.0;
        if blockers == 0 {
            continue;
        }
        // Check if any blocker is ahead (toward promotion).
        // For White, ahead = rank+1..7; for Black, ahead = 0..rank-1
        for r in 0..8 {
            let is_ahead = if color == Color::White {
                r > rank
            } else {
                r < rank
            };
            if !is_ahead {
                continue;
            }
            let rank_bb = Bitboard::rank_bb(r as u8);
            if (blockers & rank_bb.0) != 0 {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::parse_fen;

    #[test]
    fn isolated_detected() {
        // White pawn on d4 with no friendly pawn on c or e files → isolated
        let pos_iso = parse_fen("4k3/8/8/8/3P4/8/8/4K3 w - - 0 1").unwrap();
        let pos_supported = parse_fen("4k3/8/8/8/3P4/2P5/8/4K3 w - - 0 1").unwrap();
        let iso = eval(&pos_iso);
        let sup = eval(&pos_supported);
        // Isolated should be more negative (penalty)
        assert!(
            iso.0 < sup.0 || iso.1 < sup.1,
            "isolated {iso:?} vs supported {sup:?}"
        );
    }

    #[test]
    fn doubled_detected() {
        let pos_doubled = parse_fen("4k3/8/8/8/3P4/3P4/8/4K3 w - - 0 1").unwrap();
        let pos_single = parse_fen("4k3/8/8/8/3P4/8/2P5/4K3 w - - 0 1").unwrap();
        let d = eval(&pos_doubled);
        let s = eval(&pos_single);
        assert!(d.0 < s.0, "doubled {d:?} should be < single {s:?}");
    }

    #[test]
    fn passed_scales_with_rank() {
        let pos_far = parse_fen("4k3/3P4/8/8/8/8/8/4K3 w - - 0 1").unwrap(); // pawn on d7
        let pos_near = parse_fen("4k3/8/8/8/8/3P4/8/4K3 w - - 0 1").unwrap(); // pawn on d3
        let far = eval(&pos_far);
        let near = eval(&pos_near);
        assert!(far.0 > near.0, "far {far:?} vs near {near:?} MG");
        assert!(far.1 > near.1, "far {far:?} vs near {near:?} EG");
        // EG bonus grows faster toward promotion
        assert!(far.1 - near.1 > far.0 - near.0);
    }

    #[test]
    fn passed_blocked_not_counted() {
        // White pawn on d4, black pawn on d5 (same file ahead) → not passed
        let pos_blocked = parse_fen("4k3/8/8/3p4/3P4/8/8/4K3 w - - 0 1").unwrap();
        let pos_free = parse_fen("4k3/8/8/8/3P4/8/8/4K3 w - - 0 1").unwrap();
        let b = eval(&pos_blocked);
        let f = eval(&pos_free);
        assert!(
            f.0 > b.0 || f.1 > b.1,
            "free {f:?} should beat blocked {b:?}"
        );
    }
}

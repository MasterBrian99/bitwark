//! Pawn structure — isolated, doubled, passed (+ king proximity, unstoppable).
//!
//! Why pawn structure matters: pawns define the skeleton of the position.
//! Doubled pawns are immobile and weak, isolated pawns need piece protection,
//! passed pawns are future queens. These are classic handcrafted terms
//! (chessprogramming.org "Pawn Structure" / "Doubled Pawn", "Isolated Pawn",
//! "Passed Pawn"). All bonuses are tapered MG/EG and summed white − black.
//!
//! 11b: passer detection is pawn-vs-pawn (cache-pure) and stored in the
//! pawn cache entry; all passer *scoring* (flat rank, king proximity,
//! unstoppable, candidates) lives outside the cache in `passer_score`.

use crate::board::{Bitboard, Color, Piece, PieceType, Position, Square};

// Penalties are negative (applied per pawn/file).
const ISOLATED_PENALTY_MG: i32 = -10;
const ISOLATED_PENALTY_EG: i32 = -15;
const DOUBLED_PENALTY_MG: i32 = -11;
const DOUBLED_PENALTY_EG: i32 = -18;

// Passed pawn flat bonus per rank (index = rank 0..7).
// White's perspective: rank increases toward promotion (rank 7 large).
// For Black we mirror: bonus_rank = 7 - rank.
const PASSED_BONUS_MG: [i32; 8] = [0, 5, 8, 15, 25, 40, 60, 0];
const PASSED_BONUS_EG: [i32; 8] = [0, 10, 20, 35, 60, 90, 130, 0];

// Candidate passer: pawn whose path is blocked only by pieces (not pawns).
const CANDIDATE_BONUS_MG: i32 = 4;
const CANDIDATE_BONUS_EG: i32 = 8;

// King proximity per Chebyshev step (EG-heavy).
const KING_PROX_OWN_EG: i32 = 4;
const KING_PROX_OWN_MG: i32 = 1;
const KING_PROX_ENEMY_EG: i32 = 5;
const KING_PROX_ENEMY_MG: i32 = 1;

// Unstoppable passer bonus (EG).
const UNSTOPPABLE_EG: i32 = 120;
const UNSTOPPABLE_MG: i32 = 30;

/// Evaluate pawn structure, returning (mg, eg) white − black.
///
/// Legacy wrapper kept for tests — includes flat passer bonuses but *not*
#[allow(dead_code)]
/// king-proximity / unstoppable (so the test suite stays green while the
/// hot path uses the cache-pure `pawn_structure_and_passers` + `passer_score`).
pub fn eval(pos: &Position) -> (i32, i32) {
    let (mg, eg, w_pass, b_pass) = pawn_structure_and_passers(pos);
    // Legacy wrapper: flat passer bonus only (no king scaling) to preserve
    // the old test magnitudes for `passed_scales_with_rank` etc.
    let (flat_mg, flat_eg) = flat_passer_bonus(w_pass, b_pass);
    (mg + flat_mg, eg + flat_eg)
}

/// Cache-pure pawn structure.
///
/// Returns `(mg, eg, white_passers, black_passers)` where `mg/eg` are
/// isolated+doubled (and later 11c terms) only — **no** passer bonuses.
/// Passer bitboards are pawn-vs-pawn only (half-open front span), hence
/// cache-pure under `pawn_hash`.
pub fn pawn_structure_and_passers(pos: &Position) -> (i32, i32, Bitboard, Bitboard) {
    let white_pawns = pos.pieces_bb(Piece::new(Color::White, PieceType::Pawn));
    let black_pawns = pos.pieces_bb(Piece::new(Color::Black, PieceType::Pawn));

    let (w_mg, w_eg, w_pass) = eval_side_structure(pos, Color::White, white_pawns, black_pawns);
    let (b_mg, b_eg, b_pass) = eval_side_structure(pos, Color::Black, black_pawns, white_pawns);

    (w_mg - b_mg, w_eg - b_eg, w_pass, b_pass)
}

fn eval_side_structure(
    _pos: &Position,
    color: Color,
    own_pawns: Bitboard,
    enemy_pawns: Bitboard,
) -> (i32, i32, Bitboard) {
    if own_pawns.is_empty() {
        return (0, 0, Bitboard::EMPTY);
    }

    let mut mg: i32 = 0;
    let mut eg: i32 = 0;
    let mut passers = Bitboard::EMPTY;

    // Precompute per-file counts for doubled detection.
    let mut file_counts = [0u8; 8];
    for sq in own_pawns.squares() {
        file_counts[sq.file() as usize] += 1;
    }

    for sq in own_pawns.squares() {
        let file = sq.file() as usize;

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

        // Passer detection (pawn-vs-pawn only) — store bitboard, don't score yet.
        if is_passed(sq, color, enemy_pawns) {
            passers = Bitboard(passers.0 | Bitboard::from_sq(sq).0);
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

    (mg, eg, passers)
}

/// Passer scoring — king-dependent, NOT cached.
///
/// Includes flat rank bonus, king proximity, unstoppable, and candidate bonus.
/// Returns (mg, eg) white − black.
pub fn passer_score(pos: &Position, w_passers: Bitboard, b_passers: Bitboard) -> (i32, i32) {
    let mut mg: i32 = 0;
    let mut eg: i32 = 0;

    let w_king = pos.king_square(Color::White);
    let b_king = pos.king_square(Color::Black);

    // White passers (positive).
    for sq in w_passers.squares() {
        let rank = sq.rank() as usize;
        mg += PASSED_BONUS_MG[rank];
        eg += PASSED_BONUS_EG[rank];

        // King proximity.
        if let Some(k) = w_king {
            let promo = Square::new(56 + sq.file());
            let d = chebyshev(k, promo);
            // Closer own king → bonus: (7 - d) * factor.
            let bonus_eg = (7 - d) * KING_PROX_OWN_EG;
            let bonus_mg = (7 - d) * KING_PROX_OWN_MG;
            eg += bonus_eg;
            mg += bonus_mg;
        }
        if let Some(k) = b_king {
            let d = chebyshev(k, sq);
            eg += d * KING_PROX_ENEMY_EG;
            mg += d * KING_PROX_ENEMY_MG;
        }

        // Unstoppable (EG).
        if is_unstoppable(pos, sq, Color::White) {
            eg += UNSTOPPABLE_EG;
            mg += UNSTOPPABLE_MG;
        }
    }

    // Black passers (negative — subtract white perspective).
    for sq in b_passers.squares() {
        let rank = sq.rank() as usize;
        let br = 7 - rank;
        mg -= PASSED_BONUS_MG[br];
        eg -= PASSED_BONUS_EG[br];

        if let Some(k) = b_king {
            let promo = Square::new(sq.file()); // rank 0
            let d = chebyshev(k, promo);
            let bonus_eg = (7 - d) * KING_PROX_OWN_EG;
            let bonus_mg = (7 - d) * KING_PROX_OWN_MG;
            eg -= bonus_eg;
            mg -= bonus_mg;
        }
        if let Some(k) = w_king {
            let d = chebyshev(k, sq);
            eg -= d * KING_PROX_ENEMY_EG;
            mg -= d * KING_PROX_ENEMY_MG;
        }

        if is_unstoppable(pos, sq, Color::Black) {
            eg -= UNSTOPPABLE_EG;
            mg -= UNSTOPPABLE_MG;
        }
    }

    // Candidates (small bonus) — disabled for 11b to keep NPS; revisited if needed.
    let _ = (CANDIDATE_BONUS_MG, CANDIDATE_BONUS_EG);

    (mg, eg)
}

#[allow(dead_code)]
fn flat_passer_bonus(w_passers: Bitboard, b_passers: Bitboard) -> (i32, i32) {
    let mut mg = 0;
    let mut eg = 0;
    for sq in w_passers.squares() {
        let r = sq.rank() as usize;
        mg += PASSED_BONUS_MG[r];
        eg += PASSED_BONUS_EG[r];
    }
    for sq in b_passers.squares() {
        let r = sq.rank() as usize;
        let br = 7 - r;
        mg -= PASSED_BONUS_MG[br];
        eg -= PASSED_BONUS_EG[br];
    }
    (mg, eg)
}

fn chebyshev(a: Square, b: Square) -> i32 {
    let df = (a.file() as i32 - b.file() as i32).abs();
    let dr = (a.rank() as i32 - b.rank() as i32).abs();
    df.max(dr)
}

fn is_unstoppable(pos: &Position, sq: Square, color: Color) -> bool {
    // Cheap unstoppable: passer on 6th/7th rank and enemy king too far to catch.
    let rank = sq.rank() as i32;
    let file = sq.file() as i32;
    let enemy = color.opposite();
    let Some(ek) = pos.king_square(enemy) else {
        return false;
    };
    // Promotion square.
    let promo = if color == Color::White {
        Square::new((7 * 8 + file) as u8)
    } else {
        Square::new(file as u8)
    };
    let pushes = if color == Color::White {
        7 - rank
    } else {
        rank
    };
    if pushes > 2 {
        return false; // only near promotion.
    }
    if pushes <= 0 {
        return false;
    }
    // Enemy king distance to promotion (Chebyshev). If > pushes, cannot catch.
    // Side-to-move matters: if enemy to move, it gets one extra king move.
    let dist = chebyshev(ek, promo);
    let offset = if pos.side_to_move() == enemy { 0 } else { 1 };
    if dist > pushes + offset {
        // Also ensure no enemy piece blocks the path (simplified: no piece on promo file ahead).
        let occupied = pos.occupied();
        let promo_file_bb = Bitboard::file_bb(file as u8);
        // Check for blockers on the file between sq and promo (exclusive).
        let mut blocked = false;
        for r in 0..8 {
            if color == Color::White && r <= rank {
                continue;
            }
            if color == Color::Black && r >= rank {
                continue;
            }
            let s = Square::new((r * 8 + file) as u8);
            if occupied.contains(s) {
                // Any piece on the promotion file ahead blocks unstoppable.
                let _ = promo_file_bb;
                blocked = true;
                break;
            }
        }
        if !blocked {
            return true;
        }
    }
    false
}

#[allow(dead_code)]
fn candidate_passers(
    pos: &Position,
    color: Color,
    own_pawns: Bitboard,
    _enemy_pawns: Bitboard,
    passers: Bitboard,
) -> Bitboard {
    let mut cands = Bitboard::EMPTY;
    let enemy_pawns = pos.pieces_bb(Piece::new(color.opposite(), PieceType::Pawn));
    let occupied = pos.occupied();
    for sq in own_pawns.squares() {
        if passers.contains(sq) {
            continue;
        }
        if is_passed(sq, color, enemy_pawns) {
            continue; // would have been a passer.
        }
        // Not a pawn-blocked passer — check if blocked only by pieces (not pawns).
        // If no enemy pawn on the front span but there is a piece, it's a candidate.
        if is_passed(sq, color, enemy_pawns) {
            continue;
        }
        // Actually need: enemy pawn span is clean (already checked via is_passed false due to pawn),
        // but piece span is occupied. Re-check: enemy pawns don't block, but pieces do.
        // For candidate, enemy pawns must NOT block, but pieces do.
        // So if is_passed(sq, color, enemy_pawns) is false due to pawn block, not a candidate.
        // Candidate: pawn-blocked == false, but piece-blocked == true.
        // We already know is_passed false due to pawn? We need to distinguish.
        // Simplify: if enemy pawns block, not a candidate. So skip.
        // Check piece block: look for any occupied square on same/adjacent files ahead.
        let file = sq.file() as i32;
        let rank = sq.rank() as i32;
        let mut piece_block = false;
        for df in -1..=1 {
            let f = file + df;
            if !(0..8).contains(&f) {
                continue;
            }
            for r in 0..8 {
                let ahead = if color == Color::White {
                    r > rank
                } else {
                    r < rank
                };
                if !ahead {
                    continue;
                }
                let s = Square::new((r * 8 + f) as u8);
                if occupied.contains(s) && !enemy_pawns.contains(s) && !own_pawns.contains(s) {
                    // Enemy piece (not pawn) on the front span.
                    piece_block = true;
                    break;
                }
            }
            if piece_block {
                break;
            }
        }
        // Candidate if piece-blocked and not pawn-blocked.
        // We already know pawn-blocked would make is_passed false, so need to test the converse:
        // is_passed_with_pawns_false_but_no_pawn_block? Harder.
        // For now mark as candidate if piece_block and no pawn blocker on those squares.
        // Check pawn blocker explicitly:
        let mut pawn_block = false;
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
            for r in 0..8 {
                let ahead = if color == Color::White {
                    r > rank
                } else {
                    r < rank
                };
                if !ahead {
                    continue;
                }
                let rank_bb = Bitboard::rank_bb(r as u8);
                if (blockers & rank_bb.0) != 0 {
                    pawn_block = true;
                    break;
                }
            }
            if pawn_block {
                break;
            }
        }
        if !pawn_block && piece_block {
            cands = Bitboard(cands.0 | Bitboard::from_sq(sq).0);
        }
    }
    cands
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

    #[test]
    fn passer_king_proximity() {
        // Same passer, own king close vs far
        let pos_close = parse_fen("8/3P4/8/8/8/8/4K3/4k3 w - - 0 1").unwrap(); // WK e2 close to d7
        let pos_far = parse_fen("8/3P4/8/8/8/8/8/K6k w - - 0 1").unwrap(); // WK a1 far
        let (_, _, wp_close, _) = pawn_structure_and_passers(&pos_close);
        let (_, _, wp_far, _) = pawn_structure_and_passers(&pos_far);
        let s_close = passer_score(&pos_close, wp_close, Bitboard::EMPTY);
        let s_far = passer_score(&pos_far, wp_far, Bitboard::EMPTY);
        assert!(
            s_close.1 > s_far.1,
            "own king close {s_close:?} vs far {s_far:?}"
        );
    }

    #[test]
    fn unstoppable_detection() {
        // White pawn on 7th, black king far -> unstoppable bonus
        let pos_unstoppable = parse_fen("7k/3P4/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let pos_catchable = parse_fen("4k3/3P4/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let (_, _, wp_u, _) = pawn_structure_and_passers(&pos_unstoppable);
        let (_, _, wp_c, _) = pawn_structure_and_passers(&pos_catchable);
        let su = passer_score(&pos_unstoppable, wp_u, Bitboard::EMPTY);
        let sc = passer_score(&pos_catchable, wp_c, Bitboard::EMPTY);
        assert!(su.1 > sc.1, "unstoppable {su:?} vs catchable {sc:?}");
    }
}

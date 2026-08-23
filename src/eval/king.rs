//! King safety — pawn shield + king-ring attack units.
//!
//! Pawn shield counts own pawns on the three files around the king in the
//! two ranks ahead (middlegame only). King-ring attack scoring counts
//! per-square attacks on the king's zone (3 files × 3 ranks toward the
//! enemy) weighted by piece type, requires ≥2 attackers, and looks up a
//! nonlinear ~100-entry safety table.
//! Classical handcrafted approach (chessprogramming.org "King Safety",
//! "King Safety (Attack Units)").

use crate::board::{
    Bitboard, Color, Piece, PieceType, Position, Square,
    attacks::{bishop_attacks, king_attacks, knight_attacks, queen_attacks, rook_attacks},
};

// Shield bonus per pawn, MG only, tapered.
const SHIELD_FIRST_RANK: i32 = 10;
const SHIELD_SECOND_RANK: i32 = 5;

// Per-square attack weights (distinct per piece type).
const ATTACK_WEIGHT_N: i32 = 2;
const ATTACK_WEIGHT_B: i32 = 2;
const ATTACK_WEIGHT_R: i32 = 3;
const ATTACK_WEIGHT_Q: i32 = 5;

// Nonlinear safety table — MG penalties (negative) inflicted on the defender.
// Indexed by attack_weight (sum of per-square weighted hits). Requires ≥2
// attackers to activate; otherwise penalty is 0.
// Shape: quadratic ramp `- (i*i)/4 - i*2` capped around -900 at i=99.
// Handcrafted starting point for Phase 12 tuning.
const SAFETY_TABLE: [i32; 100] = [
    0, -2, -5, -10, -16, -24, -34, -46, -60, -76, -94, -114, -136, -160, -186, -214, -244, -276,
    -310, -346, -384, -424, -466, -510, -556, -604, -654, -706, -760, -760, -760, -760, -760, -760,
    -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760,
    -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760,
    -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760,
    -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760, -760,
    -760, -760,
];

/// King shield (mg, eg) white − black. EG ≈ 0 (MG concept, tapered).
pub fn shield(pos: &Position) -> (i32, i32) {
    let w = shield_side(pos, Color::White);
    let b = shield_side(pos, Color::Black);
    (w.0 - b.0, w.1 - b.1)
}

/// King attack safety (mg, eg) white − black. EG ≈ 0.
pub fn attack(pos: &Position) -> (i32, i32) {
    let w = attack_side(pos, Color::White);
    let b = attack_side(pos, Color::Black);
    (w.0 - b.0, w.1 - b.1)
}

/// Combined king safety (mg, eg) white − black — shield + attack.
/// Kept for backward-compat tests; new code should use shield()+attack() separately
/// for breakdown rows.
#[allow(dead_code)]
pub fn eval(pos: &Position) -> (i32, i32) {
    let (smg, seg) = shield(pos);
    let (amg, aeg) = attack(pos);
    (smg + amg, seg + aeg)
}

fn shield_side(pos: &Position, color: Color) -> (i32, i32) {
    let Some(king_sq) = pos.king_square(color) else {
        return (0, 0);
    };
    let own_pawns = pos.pieces_bb(Piece::new(color, PieceType::Pawn));
    let bonus = pawn_shield(king_sq, color, own_pawns);
    (bonus, 0)
}

fn attack_side(pos: &Position, color: Color) -> (i32, i32) {
    let Some(king_sq) = pos.king_square(color) else {
        return (0, 0);
    };
    let enemy = color.opposite();
    let (weight, attackers) = king_attack_weight_and_count(pos, king_sq, enemy);
    if attackers < 2 {
        return (0, 0);
    }
    let idx = (weight as usize).min(SAFETY_TABLE.len() - 1);
    let safety = SAFETY_TABLE[idx];
    (safety, 0)
}

fn pawn_shield(king_sq: Square, color: Color, own_pawns: Bitboard) -> i32 {
    let file = king_sq.file() as i32;
    let rank = king_sq.rank() as i32;
    let mut bonus = 0;

    for df in -1..=1 {
        let f = file + df;
        if !(0..8).contains(&f) {
            continue;
        }
        // Two ranks ahead toward enemy side.
        for dr in 1..=2 {
            let r = if color == Color::White {
                rank + dr
            } else {
                rank - dr
            };
            if !(0..8).contains(&r) {
                continue;
            }
            let sq = Square::new((r * 8 + f) as u8);
            if own_pawns.contains(sq) {
                let add = if dr == 1 {
                    SHIELD_FIRST_RANK
                } else {
                    SHIELD_SECOND_RANK
                };
                bonus += add;
            }
        }
    }
    bonus
}

/// King zone: 3 files × 3 ranks block centered on king file and extending
/// two ranks toward the enemy (king's own rank + 2 ahead).
fn king_zone(king_sq: Square, color: Color) -> Bitboard {
    let file = king_sq.file() as i32;
    let rank = king_sq.rank() as i32;
    let mut bb = Bitboard::EMPTY;
    for df in -1..=1 {
        let f = file + df;
        if !(0..8).contains(&f) {
            continue;
        }
        for dr in 0..=2 {
            let r = if color == Color::White {
                rank + dr
            } else {
                rank - dr
            };
            if !(0..8).contains(&r) {
                continue;
            }
            let sq = Square::new((r * 8 + f) as u8);
            bb = Bitboard(bb.0 | Bitboard::from_sq(sq).0);
        }
    }
    bb
}

fn king_attack_weight_and_count(pos: &Position, king_sq: Square, attacker: Color) -> (i32, i32) {
    let occupied = pos.occupied();
    let zone = king_zone(king_sq, attacker.opposite());
    // Also ensure king square itself is included (it is, dr=0).
    let _ = king_attacks; // keep import used if zone construction changes

    let mut weight: i32 = 0;
    let mut attackers: i32 = 0;

    // Knights
    for sq in pos
        .pieces_bb(Piece::new(attacker, PieceType::Knight))
        .squares()
    {
        let hits = (knight_attacks(sq).0 & zone.0).count_ones() as i32;
        if hits > 0 {
            weight += hits * ATTACK_WEIGHT_N;
            attackers += 1;
        }
    }
    // Bishops
    for sq in pos
        .pieces_bb(Piece::new(attacker, PieceType::Bishop))
        .squares()
    {
        let hits = (bishop_attacks(sq, occupied).0 & zone.0).count_ones() as i32;
        if hits > 0 {
            weight += hits * ATTACK_WEIGHT_B;
            attackers += 1;
        }
    }
    // Rooks
    for sq in pos
        .pieces_bb(Piece::new(attacker, PieceType::Rook))
        .squares()
    {
        let hits = (rook_attacks(sq, occupied).0 & zone.0).count_ones() as i32;
        if hits > 0 {
            weight += hits * ATTACK_WEIGHT_R;
            attackers += 1;
        }
    }
    // Queens
    for sq in pos
        .pieces_bb(Piece::new(attacker, PieceType::Queen))
        .squares()
    {
        let hits = (queen_attacks(sq, occupied).0 & zone.0).count_ones() as i32;
        if hits > 0 {
            weight += hits * ATTACK_WEIGHT_Q;
            attackers += 1;
        }
    }

    (weight, attackers)
}

/// Endgame king centralization (EG-heavy), scaled by phase.
///
/// MG ≈ 0 so middlegame castling isn't penalized. EG bonus tapers from 0 at
/// phase 24 to full at phase 0.
pub fn king_activity(pos: &Position) -> (i32, i32) {
    let phase = crate::eval::game_phase(pos);
    // Centralization table: 0 at corners, ~25 at center (d4,e4,d5,e5).
    // Based on Chebyshev distance from center.
    let w = pos.king_square(Color::White);
    let b = pos.king_square(Color::Black);
    let w_central = w.map(central_bonus).unwrap_or(0);
    let b_central = b.map(central_bonus).unwrap_or(0);
    let eg = (w_central - b_central) * (24 - phase) / 24;
    (0, eg)
}

fn central_bonus(sq: Square) -> i32 {
    let file = sq.file() as i32;
    let rank = sq.rank() as i32;
    // Distance from nearest center (d4=3,3 ; e4=4,3 ; d5=3,4 ; e5=4,4)
    let df = (file - 3).abs().min((file - 4).abs());
    let dr = (rank - 3).abs().min((rank - 4).abs());
    let dist = df.max(dr); // Chebyshev
    // 0 dist => 20, 1 => 12, 2=>6, 3=>0
    match dist {
        0 => 20,
        1 => 12,
        2 => 6,
        3 => 0,
        _ => -4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::parse_fen;

    #[test]
    fn pawn_shield_bonus() {
        // White king on g1 with pawns on f2,g2,h2 -> shield bonus
        let pos_shield = parse_fen("4k3/8/8/8/8/8/5PPP/5RK1 w - - 0 1").unwrap();
        let pos_naked = parse_fen("4k3/8/8/8/8/8/8/5RK1 w - - 0 1").unwrap();
        let s = shield(&pos_shield);
        let n = shield(&pos_naked);
        assert!(s.0 > n.0, "shield {s:?} vs naked {n:?}");
    }

    #[test]
    fn attacker_penalty() {
        // White king on e1, black queen close attacking zone (needs 2 attackers to activate)
        // Queen + bishop both attacking
        let pos_attacked = parse_fen("4k3/8/8/4q3/2b5/8/8/4K3 w - - 0 1").unwrap();
        let pos_safe = parse_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let a = attack(&pos_attacked);
        let s = attack(&pos_safe);
        assert!(a.0 < s.0, "attacked {a:?} vs safe {s:?}");
    }

    #[test]
    fn two_attacker_rule() {
        // Single queen attacking zone should give 0 (needs >=2)
        let pos_single = parse_fen("4k3/8/8/4q3/8/8/8/4K3 w - - 0 1").unwrap();
        let pos_double = parse_fen("4k3/8/8/4q3/2b5/8/8/4K3 w - - 0 1").unwrap();
        let single = attack(&pos_single);
        let double = attack(&pos_double);
        assert_eq!(single.0, 0, "single attacker should be 0, got {single:?}");
        assert!(
            double.0 < 0,
            "double attacker should be negative {double:?}"
        );
    }

    #[test]
    fn per_square_not_per_piece() {
        // Queen that hits 3 zone squares should score more negative than queen hitting 1
        // White king e1, black queen positions chosen for zone e1's 3-rank block
        // Use queen on e2 (hits many zone squares) vs queen on a5 (hits fewer)
        let pos_many = parse_fen("4k3/8/8/8/8/8/4q3/4K3 w - - 0 1").unwrap();
        let pos_few = parse_fen("4k3/8/8/q7/8/8/8/4K3 w - - 0 1").unwrap();
        // Both need a second attacker to activate
        let pos_many2 = parse_fen("4k3/8/8/8/8/2b5/4q3/4K3 w - - 0 1").unwrap();
        let pos_few2 = parse_fen("4k3/8/8/q7/8/2b5/8/4K3 w - - 0 1").unwrap();
        let many = attack(&pos_many2);
        let few = attack(&pos_few2);
        let _ = (pos_many, pos_few); // keep single-attacker cases for reference
        assert!(many.0 <= few.0, "many {many:?} should be <= few {few:?}");
    }

    #[test]
    fn no_panic_kingless() {
        let pos = crate::board::Position::empty_for_fen_internal();
        let _ = eval(&pos);
        let _ = shield(&pos);
        let _ = attack(&pos);
    }

    #[test]
    fn safety_table_monotonic() {
        for i in 1..SAFETY_TABLE.len() {
            assert!(
                SAFETY_TABLE[i] <= SAFETY_TABLE[i - 1],
                "table not monotonic at {i}: {} > {}",
                SAFETY_TABLE[i],
                SAFETY_TABLE[i - 1]
            );
        }
        assert_eq!(SAFETY_TABLE[0], 0);
    }

    #[test]
    fn king_zone_size() {
        // King on e1 -> 3 files (d,e,f) × 3 ranks (1,2,3) = 9
        let z_e1 = king_zone(Square::new(4), Color::White);
        assert_eq!(z_e1.count() as i32, 9, "e1 zone {:?}", z_e1);
        // King on a1 -> files a,b × 3 ranks = 6
        let z_a1 = king_zone(Square::new(0), Color::White);
        assert_eq!(z_a1.count() as i32, 6, "a1 zone {:?}", z_a1);
        // King on e8 black -> 9 as well
        let z_e8 = king_zone(Square::new(60), Color::Black);
        assert_eq!(z_e8.count() as i32, 9, "e8 zone {:?}", z_e8);
    }
}

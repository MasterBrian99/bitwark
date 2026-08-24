//! King safety — pawn shield + king-zone attacker count.
//!
//! Pawn shield counts own pawns on the three files around the king in the
//! two ranks ahead (middlegame only). Attacker count weights enemy
//! N/B/R/Q attacks on the king zone (king attacks plus the king square)
//! with attack units N/B=2 R=3 Q=5, looked up in a safety table.
//! Classical handcrafted approach (chessprogramming.org "King Safety").

use crate::board::{
    Bitboard, Color, Piece, PieceType, Position, Square,
    attacks::{bishop_attacks, king_attacks, knight_attacks, queen_attacks, rook_attacks},
};

// Shield bonus per pawn, MG only, tapered.
const SHIELD_FIRST_RANK: i32 = 10;
const SHIELD_SECOND_RANK: i32 = 5;

// Safety table indexed by attack_units (0..8+). Values are MG penalties
// (negative) inflicted on the defender. Tuned to be small so the term
// doesn't dominate.
const SAFETY_TABLE: [i32; 9] = [0, -2, -10, -25, -50, -80, -120, -170, -230];

/// King safety (mg, eg) white − black. EG ≈ 0 (MG concept).
pub fn eval(pos: &Position) -> (i32, i32) {
    let w = eval_side(pos, Color::White);
    let b = eval_side(pos, Color::Black);
    (w.0 - b.0, w.1 - b.1)
}

fn eval_side(pos: &Position, color: Color) -> (i32, i32) {
    let Some(king_sq) = pos.king_square(color) else {
        return (0, 0);
    };
    let _ = pos; // avoid unused if no king? already used

    let own_pawns = pos.pieces_bb(Piece::new(color, PieceType::Pawn));
    let shield = pawn_shield(king_sq, color, own_pawns);

    let enemy = color.opposite();
    let attack_units = king_attack_units(pos, king_sq, enemy);
    let idx = (attack_units as usize).min(SAFETY_TABLE.len() - 1);
    let safety = SAFETY_TABLE[idx];

    // Shield is bonus, safety is penalty (negative). Combine.
    (shield + safety, 0)
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

fn king_attack_units(pos: &Position, king_sq: Square, attacker: Color) -> i32 {
    let occupied = pos.occupied();
    let zone = Bitboard(king_attacks(king_sq).0 | Bitboard::from_sq(king_sq).0);

    let mut units: i32 = 0;

    // Knights
    for sq in pos
        .pieces_bb(Piece::new(attacker, PieceType::Knight))
        .squares()
    {
        if (knight_attacks(sq).0 & zone.0) != 0 {
            units += 2;
        }
    }
    // Bishops
    for sq in pos
        .pieces_bb(Piece::new(attacker, PieceType::Bishop))
        .squares()
    {
        if (bishop_attacks(sq, occupied).0 & zone.0) != 0 {
            units += 2;
        }
    }
    // Rooks
    for sq in pos
        .pieces_bb(Piece::new(attacker, PieceType::Rook))
        .squares()
    {
        if (rook_attacks(sq, occupied).0 & zone.0) != 0 {
            units += 3;
        }
    }
    // Queens
    for sq in pos
        .pieces_bb(Piece::new(attacker, PieceType::Queen))
        .squares()
    {
        if (queen_attacks(sq, occupied).0 & zone.0) != 0 {
            units += 5;
        }
    }
    // Note: enemy pawn attacks on zone are not counted as attack units;
    // they weaken the shield instead (shield already accounts for pawn
    // presence). This keeps the term simple.

    units
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
        let s = eval(&pos_shield);
        let n = eval(&pos_naked);
        assert!(s.0 > n.0, "shield {s:?} vs naked {n:?}");
    }

    #[test]
    fn attacker_penalty() {
        // White king on e1, black queen close attacking zone
        let pos_attacked = parse_fen("4k3/8/8/4q3/8/8/8/4K3 w - - 0 1").unwrap();
        let pos_safe = parse_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let a = eval(&pos_attacked);
        let s = eval(&pos_safe);
        assert!(a.0 < s.0, "attacked {a:?} vs safe {s:?}");
    }

    #[test]
    fn no_panic_kingless() {
        let pos = crate::board::Position::empty_for_fen_internal();
        let _ = eval(&pos);
    }
}

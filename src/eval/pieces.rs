//! Piece terms — mobility, rook open files, bishop pair.
//!
//! Mobility counts pseudo-legal attack squares not occupied by own pieces and
//! not defended by enemy pawns (chessprogramming.org "Mobility"). Rook on an
//! open/semi-open file and the bishop pair are classical bonuses
//! (chessprogramming.org "Rook on Open File", "Bishop Pair").

use crate::board::{
    Bitboard, Color, Piece, PieceType, Position,
    attacks::{bishop_attacks, knight_attacks, pawn_attacks, queen_attacks, rook_attacks},
};

// Weights per mobility square (MG, EG). Small so PeSTO PSTs aren't double-counted.
const MOBILITY_N_MG: i32 = 4;
const MOBILITY_N_EG: i32 = 4;
const MOBILITY_B_MG: i32 = 4;
const MOBILITY_B_EG: i32 = 4;
const MOBILITY_R_MG: i32 = 2;
const MOBILITY_R_EG: i32 = 2;
const MOBILITY_Q_MG: i32 = 1;
const MOBILITY_Q_EG: i32 = 1;

// Rook file bonuses (white − black). Symmetric.
const ROOK_OPEN_MG: i32 = 25;
const ROOK_OPEN_EG: i32 = 12;
const ROOK_SEMI_MG: i32 = 12;
const ROOK_SEMI_EG: i32 = 6;

// Bishop pair bonus (side with >=2 bishops).
const BISHOP_PAIR_MG: i32 = 20;
const BISHOP_PAIR_EG: i32 = 40;

/// Mobility (mg, eg) white − black.
pub fn mobility(pos: &Position) -> (i32, i32) {
    let occupied = pos.occupied();
    let white_occ = pos.occupied_color(Color::White);
    let black_occ = pos.occupied_color(Color::Black);
    let white_pawn_attacks = pawn_attacks_bb(pos, Color::White);
    let black_pawn_attacks = pawn_attacks_bb(pos, Color::Black);

    let mut mg: i32 = 0;
    let mut eg: i32 = 0;

    for &color in &[Color::White, Color::Black] {
        let (own_occ, enemy_pawn_attacks) = if color == Color::White {
            (white_occ, black_pawn_attacks)
        } else {
            (black_occ, white_pawn_attacks)
        };
        let sign = if color == Color::White { 1 } else { -1 };

        // Knights
        for sq in pos
            .pieces_bb(Piece::new(color, PieceType::Knight))
            .squares()
        {
            let attacks = knight_attacks(sq);
            let mob = (attacks.0 & !own_occ.0 & !enemy_pawn_attacks.0).count_ones() as i32;
            mg += sign * mob * MOBILITY_N_MG;
            eg += sign * mob * MOBILITY_N_EG;
        }
        // Bishops
        for sq in pos
            .pieces_bb(Piece::new(color, PieceType::Bishop))
            .squares()
        {
            let attacks = bishop_attacks(sq, occupied);
            let mob = (attacks.0 & !own_occ.0 & !enemy_pawn_attacks.0).count_ones() as i32;
            mg += sign * mob * MOBILITY_B_MG;
            eg += sign * mob * MOBILITY_B_EG;
        }
        // Rooks
        for sq in pos.pieces_bb(Piece::new(color, PieceType::Rook)).squares() {
            let attacks = rook_attacks(sq, occupied);
            let mob = (attacks.0 & !own_occ.0 & !enemy_pawn_attacks.0).count_ones() as i32;
            mg += sign * mob * MOBILITY_R_MG;
            eg += sign * mob * MOBILITY_R_EG;
        }
        // Queens
        for sq in pos.pieces_bb(Piece::new(color, PieceType::Queen)).squares() {
            let attacks = queen_attacks(sq, occupied);
            let mob = (attacks.0 & !own_occ.0 & !enemy_pawn_attacks.0).count_ones() as i32;
            mg += sign * mob * MOBILITY_Q_MG;
            eg += sign * mob * MOBILITY_Q_EG;
        }
    }

    (mg, eg)
}

/// Rook on open/semi-open file bonus, white − black.
pub fn rook_files(pos: &Position) -> (i32, i32) {
    let white_pawns = pos.pieces_bb(Piece::new(Color::White, PieceType::Pawn));
    let black_pawns = pos.pieces_bb(Piece::new(Color::Black, PieceType::Pawn));

    let mut mg: i32 = 0;
    let mut eg: i32 = 0;

    for &color in &[Color::White, Color::Black] {
        let (own_pawns, enemy_pawns) = if color == Color::White {
            (white_pawns, black_pawns)
        } else {
            (black_pawns, white_pawns)
        };
        let sign = if color == Color::White { 1 } else { -1 };
        for sq in pos.pieces_bb(Piece::new(color, PieceType::Rook)).squares() {
            let file = sq.file();
            let file_bb = Bitboard::file_bb(file);
            let has_own = (own_pawns.0 & file_bb.0) != 0;
            let has_enemy = (enemy_pawns.0 & file_bb.0) != 0;
            if !has_own && !has_enemy {
                mg += sign * ROOK_OPEN_MG;
                eg += sign * ROOK_OPEN_EG;
            } else if !has_own && has_enemy {
                mg += sign * ROOK_SEMI_MG;
                eg += sign * ROOK_SEMI_EG;
            }
        }
    }
    (mg, eg)
}

/// Bishop pair bonus, white − black. +bonus if side has >=2 bishops.
pub fn bishop_pair(pos: &Position) -> (i32, i32) {
    let white_bishops = pos
        .pieces_bb(Piece::new(Color::White, PieceType::Bishop))
        .count() as i32;
    let black_bishops = pos
        .pieces_bb(Piece::new(Color::Black, PieceType::Bishop))
        .count() as i32;
    let mut mg = 0;
    let mut eg = 0;
    if white_bishops >= 2 {
        mg += BISHOP_PAIR_MG;
        eg += BISHOP_PAIR_EG;
    }
    if black_bishops >= 2 {
        mg -= BISHOP_PAIR_MG;
        eg -= BISHOP_PAIR_EG;
    }
    (mg, eg)
}

fn pawn_attacks_bb(pos: &Position, color: Color) -> Bitboard {
    let mut bb = Bitboard::EMPTY;
    for sq in pos.pieces_bb(Piece::new(color, PieceType::Pawn)).squares() {
        bb = Bitboard(bb.0 | pawn_attacks(sq, color).0);
    }
    bb
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::parse_fen;

    #[test]
    fn rook_open_vs_closed() {
        // White rook on a1, file a with no pawns => open; vs file with pawns => closed
        let pos_open = parse_fen("4k3/8/8/8/8/8/8/R3K3 w - - 0 1").unwrap();
        let pos_closed = parse_fen("4k3/8/8/8/8/8/P7/R3K3 w - - 0 1").unwrap();
        let open = rook_files(&pos_open);
        let closed = rook_files(&pos_closed);
        assert!(open.0 > closed.0, "open {open:?} vs closed {closed:?}");
    }

    #[test]
    fn bishop_pair_bonus() {
        let pos_pair = parse_fen("4k3/8/8/8/8/8/8/BBK5 w - - 0 1").unwrap();
        let pos_single = parse_fen("4k3/8/8/8/8/8/8/B1K5 w - - 0 1").unwrap();
        let p = bishop_pair(&pos_pair);
        let s = bishop_pair(&pos_single);
        assert!(p.0 > s.0, "pair {p:?} vs single {s:?}");
        // Symmetry
        let pos_black_pair = parse_fen("4k3/8/8/8/8/8/8/4K2b w - - 0 1").unwrap();
        let pos_black_two = parse_fen("bb2k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let b = bishop_pair(&pos_black_two);
        assert!(b.0 < 0, "black pair should be negative {b:?}");
        let _ = pos_black_pair;
    }

    #[test]
    fn mobility_center_vs_corner() {
        // Bishop on d4 should have more mobility than on a1
        let pos_center = parse_fen("4k3/8/8/8/3B4/8/8/4K3 w - - 0 1").unwrap();
        let pos_corner = parse_fen("4k3/8/8/8/8/8/8/B3K3 w - - 0 1").unwrap();
        let c = mobility(&pos_center);
        let r = mobility(&pos_corner);
        assert!(c.0 > r.0, "center {c:?} vs corner {r:?}");
    }
}

#![allow(unused)]
//! Move generation — legal moves via pseudo-legal + king safety filter.
//!
//! For Phase 2 we keep the implementation simple and obviously correct:
//! generate all pseudo-legal moves (including those that leave the king in
//! check), then for each move do `make_move` + `is_attacked(king)` + `unmake`.
//! This is slower than a pin-mask approach but correct and fast enough for
//! perft — bulk counting at depth 1 recovers most of the cost.
//!
//! See chessprogramming.org “Move Generation” and “Perft”.

use crate::board::{
    attacks::{
        bishop_attacks, king_attacks, knight_attacks, pawn_attacks, queen_attacks, rook_attacks,
    },
    mv::Move,
    position::{CASTLE_BK, CASTLE_BQ, CASTLE_WK, CASTLE_WQ, Position},
    types::{Bitboard, Color, Piece, PieceType, Square},
};

/// Check if `sq` is attacked by `attacker` color in `pos`.
pub fn is_square_attacked(pos: &Position, sq: Square, attacker: Color) -> bool {
    let occupied = pos.occupied();
    let attacker_occ = pos.occupied_color(attacker);

    // Pawns: reverse pawn attacks (if pawn on sq would attack attacker pawn)
    // Actually we need to check if any attacker pawn attacks sq: pawn_attacks(sq, defender) & attacker pawns
    // Pawn attacks are from pawn square to target; to test if sq is attacked by pawn, check pawns that could attack sq.
    let pawns = pos.pieces_bb(Piece::new(attacker, PieceType::Pawn));
    if (pawn_attacks(sq, attacker.opposite()) & pawns).0 != 0 {
        return true;
    }

    // Knights
    let knights = pos.pieces_bb(Piece::new(attacker, PieceType::Knight));
    if (knight_attacks(sq) & knights).0 != 0 {
        return true;
    }

    // King
    let king = pos.pieces_bb(Piece::new(attacker, PieceType::King));
    if (king_attacks(sq) & king).0 != 0 {
        return true;
    }

    // Bishops + queens (diagonal)
    let bishops = pos.pieces_bb(Piece::new(attacker, PieceType::Bishop))
        | pos.pieces_bb(Piece::new(attacker, PieceType::Queen));
    if (bishop_attacks(sq, occupied) & bishops).0 != 0 {
        return true;
    }

    // Rooks + queens (orthogonal)
    let rooks = pos.pieces_bb(Piece::new(attacker, PieceType::Rook))
        | pos.pieces_bb(Piece::new(attacker, PieceType::Queen));
    if (rook_attacks(sq, occupied) & rooks).0 != 0 {
        return true;
    }

    false
}

/// Generate all pseudo-legal moves (may leave king in check).
pub fn generate_pseudo_legal(pos: &Position, moves: &mut Vec<Move>) {
    let us = pos.side_to_move();
    let them = us.opposite();
    let occupied = pos.occupied();
    let us_occ = pos.occupied_color(us);
    let them_occ = pos.occupied_color(them);
    let empty_or_them = !us_occ;

    let king_sq = match pos.king_square(us) {
        Some(s) => s,
        None => return,
    };

    // Pawns
    let pawns = pos.pieces_bb(Piece::new(us, PieceType::Pawn));
    for from in pawns.squares() {
        let dir: i8 = if us == Color::White { 1 } else { -1 };
        let r = from.rank() as i8;
        let f = from.file() as i8;
        let promo_rank = if us == Color::White { 7 } else { 0 };
        let start_rank = if us == Color::White { 1 } else { 6 };

        // Single push
        let nr = r + dir;
        if (0..8).contains(&nr) {
            let to = Square::from_coords(f as u8, nr as u8);
            if pos.piece_at(to).is_none() {
                if nr as u8 == promo_rank {
                    for promo in [
                        PieceType::Queen,
                        PieceType::Rook,
                        PieceType::Bishop,
                        PieceType::Knight,
                    ] {
                        moves.push(Move::new(from, to, Some(promo)));
                    }
                } else {
                    moves.push(Move::new(from, to, None));
                    // Double push
                    if r == start_rank {
                        let nr2 = r + 2 * dir;
                        let to2 = Square::from_coords(f as u8, nr2 as u8);
                        if pos.piece_at(to2).is_none() {
                            moves.push(Move::new(from, to2, None));
                        }
                    }
                }
            }
        }

        // Captures (including en passant and promotions)
        for df in [-1, 1] {
            let nf = f + df;
            let nr = r + dir;
            if !(0..8).contains(&nr) || !(0..8).contains(&nf) {
                continue;
            }
            let to = Square::from_coords(nf as u8, nr as u8);
            let target = pos.piece_at(to);
            let is_capture = target.is_some() && target.unwrap().color() == them;
            let is_ep = pos.en_passant() == Some(to) && target.is_none();

            if is_capture || is_ep {
                if nr as u8 == promo_rank {
                    for promo in [
                        PieceType::Queen,
                        PieceType::Rook,
                        PieceType::Bishop,
                        PieceType::Knight,
                    ] {
                        moves.push(Move::new(from, to, Some(promo)));
                    }
                } else {
                    moves.push(Move::new(from, to, None));
                }
            }
        }
    }

    // Knights
    let knights = pos.pieces_bb(Piece::new(us, PieceType::Knight));
    for from in knights.squares() {
        let attacks = knight_attacks(from) & empty_or_them;
        for to in attacks.squares() {
            moves.push(Move::new(from, to, None));
        }
    }

    // Bishops
    let bishops = pos.pieces_bb(Piece::new(us, PieceType::Bishop));
    for from in bishops.squares() {
        let attacks = bishop_attacks(from, occupied) & empty_or_them;
        for to in attacks.squares() {
            moves.push(Move::new(from, to, None));
        }
    }

    // Rooks
    let rooks = pos.pieces_bb(Piece::new(us, PieceType::Rook));
    for from in rooks.squares() {
        let attacks = rook_attacks(from, occupied) & empty_or_them;
        for to in attacks.squares() {
            moves.push(Move::new(from, to, None));
        }
    }

    // Queens
    let queens = pos.pieces_bb(Piece::new(us, PieceType::Queen));
    for from in queens.squares() {
        let attacks = queen_attacks(from, occupied) & empty_or_them;
        for to in attacks.squares() {
            moves.push(Move::new(from, to, None));
        }
    }

    // King (non-castling)
    let kings = pos.pieces_bb(Piece::new(us, PieceType::King));
    for from in kings.squares() {
        let attacks = king_attacks(from) & empty_or_them;
        for to in attacks.squares() {
            moves.push(Move::new(from, to, None));
        }
    }

    // Castling
    if us == Color::White {
        if pos.castling() & CASTLE_WK != 0 {
            // e1g1: squares f1,g1 empty, e1,f1,g1 not attacked, rook on h1
            let e1 = Square::from_str("e1").unwrap();
            let f1 = Square::from_str("f1").unwrap();
            let g1 = Square::from_str("g1").unwrap();
            let h1 = Square::from_str("h1").unwrap();
            if pos.piece_at(f1).is_none()
                && pos.piece_at(g1).is_none()
                && pos.piece_at(h1) == Some(Piece::WR)
                && !is_square_attacked(pos, e1, them)
                && !is_square_attacked(pos, f1, them)
                && !is_square_attacked(pos, g1, them)
            {
                moves.push(Move::new(e1, g1, None));
            }
        }
        if pos.castling() & CASTLE_WQ != 0 {
            let e1 = Square::from_str("e1").unwrap();
            let d1 = Square::from_str("d1").unwrap();
            let c1 = Square::from_str("c1").unwrap();
            let b1 = Square::from_str("b1").unwrap();
            let a1 = Square::from_str("a1").unwrap();
            if pos.piece_at(d1).is_none()
                && pos.piece_at(c1).is_none()
                && pos.piece_at(b1).is_none()
                && pos.piece_at(a1) == Some(Piece::WR)
                && !is_square_attacked(pos, e1, them)
                && !is_square_attacked(pos, d1, them)
                && !is_square_attacked(pos, c1, them)
            {
                moves.push(Move::new(e1, c1, None));
            }
        }
    } else {
        if pos.castling() & CASTLE_BK != 0 {
            let e8 = Square::from_str("e8").unwrap();
            let f8 = Square::from_str("f8").unwrap();
            let g8 = Square::from_str("g8").unwrap();
            let h8 = Square::from_str("h8").unwrap();
            if pos.piece_at(f8).is_none()
                && pos.piece_at(g8).is_none()
                && pos.piece_at(h8) == Some(Piece::BR)
                && !is_square_attacked(pos, e8, them)
                && !is_square_attacked(pos, f8, them)
                && !is_square_attacked(pos, g8, them)
            {
                moves.push(Move::new(e8, g8, None));
            }
        }
        if pos.castling() & CASTLE_BQ != 0 {
            let e8 = Square::from_str("e8").unwrap();
            let d8 = Square::from_str("d8").unwrap();
            let c8 = Square::from_str("c8").unwrap();
            let b8 = Square::from_str("b8").unwrap();
            let a8 = Square::from_str("a8").unwrap();
            if pos.piece_at(d8).is_none()
                && pos.piece_at(c8).is_none()
                && pos.piece_at(b8).is_none()
                && pos.piece_at(a8) == Some(Piece::BR)
                && !is_square_attacked(pos, e8, them)
                && !is_square_attacked(pos, d8, them)
                && !is_square_attacked(pos, c8, them)
            {
                moves.push(Move::new(e8, c8, None));
            }
        }
    }
}

/// Generate all legal moves (filters pseudo-legal that leave king in check).
pub fn generate_legal(pos: &mut Position, moves: &mut Vec<Move>) {
    let mut pseudo = Vec::new();
    generate_pseudo_legal(pos, &mut pseudo);
    let us = pos.side_to_move();
    for mv in pseudo {
        pos.make_move(mv);
        let king_sq = pos.king_square(us);
        let in_check = if let Some(ksq) = king_sq {
            is_square_attacked(pos, ksq, us.opposite())
        } else {
            // No king found after move? Should not happen, treat as illegal
            true
        };
        pos.unmake_move(mv);
        if !in_check {
            moves.push(mv);
        }
    }
}

/// Count legal moves (for perft bulk).
pub fn count_legal(pos: &mut Position) -> usize {
    let mut moves = Vec::new();
    generate_legal(pos, &mut moves);
    moves.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::parse_fen;

    #[test]
    fn startpos_legal_count() {
        let mut pos =
            parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let mut moves = Vec::new();
        generate_legal(&mut pos, &mut moves);
        assert_eq!(moves.len(), 20);
    }

    #[test]
    fn kiwipete_legal_count() {
        let mut pos =
            parse_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
                .unwrap();
        let mut moves = Vec::new();
        generate_legal(&mut pos, &mut moves);
        assert_eq!(moves.len(), 48);
    }

    #[test]
    fn is_attacked_basic() {
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        // e1 king not attacked at start
        let e1 = Square::from_str("e1").unwrap();
        assert!(!is_square_attacked(&pos, e1, Color::Black));
        // e2 pawn attacks d3/f3
        let d3 = Square::from_str("d3").unwrap();
        assert!(is_square_attacked(&pos, d3, Color::White));
    }
}

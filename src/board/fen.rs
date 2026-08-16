#![allow(unused)]
//! FEN (Forsyth-Edwards Notation) parsing and emission.
//!
//! FEN is the standard textual encoding of a position (see
//! chessprogramming.org “Forsyth-Edwards Notation” and `UCI spec §2.7`).
//! Six space-separated fields:
//!  1. Piece placement: ranks 8→1, `/` separated, `1..8` = empty run.
//!  2. Side to move: `w`/`b`.
//!  3. Castling rights: `KQkq` or `-` (order `KQkq` on output, any order accepted on input).
//!  4. En passant target: `-` or `[a-h][36]` (stored verbatim; capture legality is checked at make time).
//!  5. Halfmove clock (0..100).
//!  6. Fullmove number (>=1).
//!
//! Leniency: Stockfish accepts 4-field FENs (missing halfmove/fullmove); we do too for GUI compat.

use crate::board::position::{CASTLE_BK, CASTLE_BQ, CASTLE_WK, CASTLE_WQ, Position};
use crate::board::types::{Color, Piece, Square};

pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// Error from `parse_fen`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenError(pub String);

impl std::fmt::Display for FenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fen: {}", self.0)
    }
}

impl std::error::Error for FenError {}

/// Parse `fen` into a `Position`. Accepts 4 or 6 fields (4-field defaults halfmove=0 fullmove=1).
pub fn parse_fen(fen: &str) -> Result<Position, FenError> {
    let fen = fen.trim();
    let parts: Vec<&str> = fen.split_whitespace().collect();
    if parts.len() != 6 && parts.len() != 4 {
        return Err(FenError(format!(
            "expected 4 or 6 fields, got {} in '{fen}'",
            parts.len()
        )));
    }

    let placement = parts[0];
    let side = parts[1];
    let castling = parts[2];
    let en_passant = parts[3];
    let halfmove_str = if parts.len() == 6 { parts[4] } else { "0" };
    let fullmove_str = if parts.len() == 6 { parts[5] } else { "1" };

    // Use empty then fill via placements.
    // We construct via direct board manipulation to avoid double hashing.
    let mut pos = Position::empty_for_fen_internal();

    // 1. Piece placement
    let mut rank: i8 = 7;
    let mut file: i8 = 0;
    for ch in placement.chars() {
        match ch {
            '/' => {
                if file != 8 {
                    return Err(FenError(format!(
                        "rank not filled before '/' in '{placement}'"
                    )));
                }
                rank -= 1;
                if rank < 0 {
                    return Err(FenError("too many ranks".to_string()));
                }
                file = 0;
            }
            '1'..='8' => {
                let empty = (ch as u8 - b'0') as i8;
                file += empty;
                if file > 8 {
                    return Err(FenError(format!("file overflow on '{ch}'")));
                }
            }
            _ => {
                if file >= 8 {
                    return Err(FenError("file overflow on piece".to_string()));
                }
                let piece = Piece::from_char(ch)
                    .ok_or_else(|| FenError(format!("invalid piece char '{ch}'")))?;
                let sq = Square::from_coords(file as u8, rank as u8);
                pos.place_piece_for_fen(sq, piece);
                file += 1;
            }
        }
    }
    if rank != 0 || file != 8 {
        return Err(FenError(format!(
            "placement did not fill 8x8 exactly (ended rank {rank} file {file})"
        )));
    }

    // Validate kings — Stockfish requires exactly one per side.
    let wk = pos.pieces_bb(Piece::WK).count();
    let bk = pos.pieces_bb(Piece::BK).count();
    if wk != 1 || bk != 1 {
        return Err(FenError(format!(
            "expected 1 white king and 1 black king, got {wk} and {bk}"
        )));
    }

    // 2. Side to move
    match side {
        "w" => pos.set_side_to_move(Color::White),
        "b" => pos.set_side_to_move(Color::Black),
        _ => return Err(FenError(format!("side must be 'w' or 'b', got '{side}'"))),
    }

    // 3. Castling
    if castling == "-" {
        pos.set_castling(0);
    } else {
        let mut mask: u8 = 0;
        for ch in castling.chars() {
            match ch {
                'K' => mask |= CASTLE_WK,
                'Q' => mask |= CASTLE_WQ,
                'k' => mask |= CASTLE_BK,
                'q' => mask |= CASTLE_BQ,
                _ => return Err(FenError(format!("invalid castling char '{ch}'"))),
            }
        }
        // Duplicate chars are tolerated (mask idempotent).
        pos.set_castling(mask);
    }

    // 4. En passant
    if en_passant == "-" {
        pos.set_en_passant(None);
    } else {
        let sq = Square::from_str(en_passant)
            .ok_or_else(|| FenError(format!("invalid en passant square '{en_passant}'")))?;
        // Basic sanity: must be on rank 3 (white to move) or 6 (black to move) — we stay
        // lenient and store verbatim. We only check that the square is on file a..h and rank 3/6?
        // For now accept any square a1..h8 to be maximally lenient; the `d` output will echo it.
        // If we want Stockfish-like strictness, uncomment below:
        // let rank = sq.rank();
        // if rank != 2 && rank != 5 {
        //     return Err(FenError(format!("en passant square must be on rank 3 or 6, got '{en_passant}'")));
        // }
        pos.set_en_passant(Some(sq));
    }

    // 5. Halfmove
    let halfmove: u8 = halfmove_str.parse().map_err(|_| {
        FenError(format!(
            "halfmove must be integer 0..100, got '{halfmove_str}'"
        ))
    })?;
    // Stockfish clamps to 0..100; we accept 0..255 for now.
    pos.set_halfmove(halfmove);

    // 6. Fullmove
    let fullmove: u16 = fullmove_str.parse().map_err(|_| {
        FenError(format!(
            "fullmove must be integer >=1, got '{fullmove_str}'"
        ))
    })?;
    if fullmove == 0 {
        return Err(FenError("fullmove must be >=1".to_string()));
    }
    pos.set_fullmove(fullmove);

    // Recompute hash after all fields are set.
    pos.rebuild_from_board();

    Ok(pos)
}

/// Emit FEN for `pos`. Output is canonical: ranks 8→1, `KQkq` order, `-` for empty rights/ep.
pub fn to_fen(pos: &Position) -> String {
    let mut fen = String::new();

    // 1. Piece placement
    for rank in (0..8).rev() {
        let mut empty: u8 = 0;
        for file in 0..8 {
            let sq = Square::from_coords(file, rank);
            if let Some(p) = pos.piece_at(sq) {
                if empty != 0 {
                    fen.push(char::from(b'0' + empty));
                    empty = 0;
                }
                fen.push(p.to_char());
            } else {
                empty += 1;
            }
        }
        if empty != 0 {
            fen.push(char::from(b'0' + empty));
        }
        if rank != 0 {
            fen.push('/');
        }
    }

    // 2. Side
    fen.push(' ');
    fen.push(match pos.side_to_move() {
        Color::White => 'w',
        Color::Black => 'b',
    });

    // 3. Castling
    fen.push(' ');
    let c = pos.castling();
    if c == 0 {
        fen.push('-');
    } else {
        if c & CASTLE_WK != 0 {
            fen.push('K');
        }
        if c & CASTLE_WQ != 0 {
            fen.push('Q');
        }
        if c & CASTLE_BK != 0 {
            fen.push('k');
        }
        if c & CASTLE_BQ != 0 {
            fen.push('q');
        }
    }

    // 4. En passant
    fen.push(' ');
    match pos.en_passant() {
        Some(sq) => fen.push_str(&sq.to_string()),
        None => fen.push('-'),
    }

    // 5 & 6. Halfmove / Fullmove
    fen.push(' ');
    fen.push_str(&pos.halfmove().to_string());
    fen.push(' ');
    fen.push_str(&pos.fullmove().to_string());

    fen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::types::{Color, Square};

    #[test]
    fn startpos_round_trip() {
        let pos = parse_fen(START_FEN).unwrap();
        assert_eq!(to_fen(&pos), START_FEN);
    }

    #[test]
    fn valid_fens() {
        let cases = [
            START_FEN,
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1",
            "8/8/8/4k3/8/8/8/4K3 w - - 0 1",
            "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq e6 0 1",
        ];
        for fen in cases {
            let pos = parse_fen(fen).unwrap_or_else(|e| panic!("failed to parse {fen}: {e}"));
            let emitted = to_fen(&pos);
            // Re-parse emitted should give same position (FEN canonicalization).
            let pos2 = parse_fen(&emitted).unwrap();
            assert_eq!(pos, pos2, "round-trip mismatch for {fen} -> {emitted}");
        }
    }

    #[test]
    fn invalid_fens() {
        assert!(parse_fen("not a fen").is_err());
        assert!(parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0").is_err()); // 5 fields
        assert!(parse_fen("8/8/8/8/8/8/8/8 w - - 0 1").is_err()); // no kings
        assert!(parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR x KQkq - 0 1").is_err()); // bad side
        assert!(parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w X - 0 1").is_err()); // bad castling
    }

    #[test]
    fn castling_order_canonical() {
        // Input order qkQK should still emit KQkq
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w qkQK - 0 1").unwrap();
        assert_eq!(to_fen(&pos).split_whitespace().nth(2).unwrap(), "KQkq");
        // Single right
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w K - 0 1").unwrap();
        assert_eq!(to_fen(&pos).split_whitespace().nth(2).unwrap(), "K");
        // No rights
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w - - 0 1").unwrap();
        assert_eq!(to_fen(&pos).split_whitespace().nth(2).unwrap(), "-");
    }

    #[test]
    fn en_passant_round_trip() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        let pos = parse_fen(fen).unwrap();
        assert_eq!(pos.en_passant(), Some(Square::from_str("e3").unwrap()));
        assert_eq!(to_fen(&pos), fen);
        let fen2 = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let pos2 = parse_fen(fen2).unwrap();
        assert_eq!(pos2.en_passant(), None);
    }

    #[test]
    fn four_field_fen_lenient() {
        // Stockfish accepts 4-field FENs; we default halfmove/fullmove.
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq -").unwrap();
        assert_eq!(pos.halfmove(), 0);
        assert_eq!(pos.fullmove(), 1);
        let pos = parse_fen("8/5k2/8/8/8/8/4K3/8 w - -").unwrap();
        assert_eq!(to_fen(&pos).split_whitespace().count(), 6);
    }
}

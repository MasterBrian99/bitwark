#![allow(unused)]
//! Attack tables — leapers and sliders.
//!
//! # Why precompute?
//!
//! Move generation asks “what squares does this piece attack?” millions of
//! times per second. For leapers (knight/king/pawn) the answer depends only
//! on the square, so we table it once: `knight_attacks[sq]` is a `Bitboard`.
//!
//! Sliders (bishop/rook) depend on square *and* occupancy (blockers). The
//! classical trick is *magic bitboards* (fancy variant, see
//! chessprogramming.org “Magic Bitboards”): hash the blocker bits with a
//! magic number, index a precomputed table of attack sets. We generate the
//! magics at startup with a fixed-seed `Xorshift64` so the tables are
//! deterministic across runs — Stockfish does the same. Init is ~30-50 ms
//! and happens before `uciok` so the GUI never waits.
//!
//! Phase 2a: leapers only. Phase 2b: add fancy magics.

use crate::board::types::{Bitboard, Color, Square};

// ---------------------------------------------------------------------------
// Leapers — computed at first use, cached in `OnceLock`
// ---------------------------------------------------------------------------

use std::sync::OnceLock;

static KNIGHT_ATTACKS: OnceLock<[Bitboard; 64]> = OnceLock::new();
static KING_ATTACKS: OnceLock<[Bitboard; 64]> = OnceLock::new();
static PAWN_ATTACKS: OnceLock<[[Bitboard; 64]; 2]> = OnceLock::new();

fn init_knight_attacks() -> [Bitboard; 64] {
    let mut table = [Bitboard::EMPTY; 64];
    for sq in Square::ALL {
        let r = sq.rank() as i8;
        let f = sq.file() as i8;
        let deltas = [
            (2, 1),
            (1, 2),
            (-1, 2),
            (-2, 1),
            (-2, -1),
            (-1, -2),
            (1, -2),
            (2, -1),
        ];
        let mut bb = Bitboard::EMPTY;
        for (dr, df) in deltas {
            let nr = r + dr;
            let nf = f + df;
            if (0..8).contains(&nr) && (0..8).contains(&nf) {
                let nsq = Square::from_coords(nf as u8, nr as u8);
                bb |= Bitboard::from_sq(nsq);
            }
        }
        table[sq.index() as usize] = bb;
    }
    table
}

fn init_king_attacks() -> [Bitboard; 64] {
    let mut table = [Bitboard::EMPTY; 64];
    for sq in Square::ALL {
        let r = sq.rank() as i8;
        let f = sq.file() as i8;
        let mut bb = Bitboard::EMPTY;
        for dr in -1..=1 {
            for df in -1..=1 {
                if dr == 0 && df == 0 {
                    continue;
                }
                let nr = r + dr;
                let nf = f + df;
                if (0..8).contains(&nr) && (0..8).contains(&nf) {
                    let nsq = Square::from_coords(nf as u8, nr as u8);
                    bb |= Bitboard::from_sq(nsq);
                }
            }
        }
        table[sq.index() as usize] = bb;
    }
    table
}

fn init_pawn_attacks() -> [[Bitboard; 64]; 2] {
    let mut table = [[Bitboard::EMPTY; 64]; 2];
    for sq in Square::ALL {
        let r = sq.rank() as i8;
        let f = sq.file() as i8;
        // White pawns attack north
        let mut wbb = Bitboard::EMPTY;
        for df in [-1, 1] {
            let nr = r + 1;
            let nf = f + df;
            if (0..8).contains(&nr) && (0..8).contains(&nf) {
                wbb |= Bitboard::from_sq(Square::from_coords(nf as u8, nr as u8));
            }
        }
        // Black pawns attack south
        let mut bbb = Bitboard::EMPTY;
        for df in [-1, 1] {
            let nr = r - 1;
            let nf = f + df;
            if (0..8).contains(&nr) && (0..8).contains(&nf) {
                bbb |= Bitboard::from_sq(Square::from_coords(nf as u8, nr as u8));
            }
        }
        table[Color::White.as_usize()][sq.index() as usize] = wbb;
        table[Color::Black.as_usize()][sq.index() as usize] = bbb;
    }
    table
}

/// Knight attacks from `sq`.
#[inline]
pub fn knight_attacks(sq: Square) -> Bitboard {
    KNIGHT_ATTACKS.get_or_init(init_knight_attacks)[sq.index() as usize]
}

/// King attacks from `sq`.
#[inline]
pub fn king_attacks(sq: Square) -> Bitboard {
    KING_ATTACKS.get_or_init(init_king_attacks)[sq.index() as usize]
}

/// Pawn attacks from `sq` for `color` (squares a pawn on `sq` would capture to).
#[inline]
pub fn pawn_attacks(sq: Square, color: Color) -> Bitboard {
    PAWN_ATTACKS.get_or_init(init_pawn_attacks)[color.as_usize()][sq.index() as usize]
}

/// Ensure leaper tables are initialized (call before `uciok` to hide latency).
pub fn init() {
    let _ = KNIGHT_ATTACKS.get_or_init(init_knight_attacks);
    let _ = KING_ATTACKS.get_or_init(init_king_attacks);
    let _ = PAWN_ATTACKS.get_or_init(init_pawn_attacks);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::types::{E4, Square};

    #[test]
    fn knight_center() {
        // E4 knight should attack 8 squares.
        let attacks = knight_attacks(E4);
        assert_eq!(attacks.count(), 8);
        // Destinations: d6, f6, c5, g5, c3, g3, d2, f2
        for sq in ["d6", "f6", "c5", "g5", "c3", "g3", "d2", "f2"] {
            let s = Square::from_str(sq).unwrap();
            assert!(attacks.contains(s), "knight e4 should attack {sq}");
        }
    }

    #[test]
    fn knight_corner() {
        let a1 = Square::from_str("a1").unwrap();
        let attacks = knight_attacks(a1);
        assert_eq!(attacks.count(), 2);
        assert!(attacks.contains(Square::from_str("b3").unwrap()));
        assert!(attacks.contains(Square::from_str("c2").unwrap()));
    }

    #[test]
    fn king_center() {
        let e4 = Square::from_str("e4").unwrap();
        assert_eq!(king_attacks(e4).count(), 8);
        let a1 = Square::from_str("a1").unwrap();
        assert_eq!(king_attacks(a1).count(), 3);
        let h8 = Square::from_str("h8").unwrap();
        assert_eq!(king_attacks(h8).count(), 3);
    }

    #[test]
    fn pawn_attacks_basic() {
        let e4 = Square::from_str("e4").unwrap();
        let w = pawn_attacks(e4, Color::White);
        assert_eq!(w.count(), 2);
        assert!(w.contains(Square::from_str("d5").unwrap()));
        assert!(w.contains(Square::from_str("f5").unwrap()));
        let b = pawn_attacks(e4, Color::Black);
        assert_eq!(b.count(), 2);
        assert!(b.contains(Square::from_str("d3").unwrap()));
        assert!(b.contains(Square::from_str("f3").unwrap()));

        // Edge: pawn on a file attacks only one square
        let a4 = Square::from_str("a4").unwrap();
        assert_eq!(pawn_attacks(a4, Color::White).count(), 1);
        assert!(pawn_attacks(a4, Color::White).contains(Square::from_str("b5").unwrap()));
        // Pawn on 8th rank attacks none (off board)
        let e8 = Square::from_str("e8").unwrap();
        assert!(pawn_attacks(e8, Color::White).is_empty());
        let e1 = Square::from_str("e1").unwrap();
        assert!(pawn_attacks(e1, Color::Black).is_empty());
    }

    #[test]
    fn init_idempotent() {
        init();
        init();
        // Should not panic, tables remain same
        assert_eq!(knight_attacks(E4).count(), 8);
    }
}

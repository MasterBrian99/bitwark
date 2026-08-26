#![allow(unused)]
//! Fundamental chess types: `Square`, `Color`, `PieceType`, `Piece`, `Bitboard`.
//!
//! These are zero-cost wrappers around integers. The goal is type safety
//! without runtime cost — a `Square` is still just a `u8`, but you cannot
//! accidentally pass a `Piece` where a `Square` is expected.
//!
//! # Square numbering (LEFR — Little-Endian File-Rank)
//!
//! ```text
//!   56 a8 57 b8 58 c8 59 d8 60 e8 61 f8 62 g8 63 h8  ← rank 7
//!   48 a7 49 b7 50 c7 51 d7 52 e7 53 f7 54 g7 55 h7
//!   40 a6 41 b6 42 c6 43 d6 44 e6 45 f6 46 g6 47 h6
//!   32 a5 33 b5 34 c5 35 d5 36 e5 37 f5 38 g5 39 h5
//!   24 a4 25 b4 26 c4 27 d4 28 e4 29 f4 30 g4 31 h4
//!   16 a3 17 b3 18 c3 19 d3 20 e3 21 f3 22 g3 23 h3
//!    8 a2  9 b2 10 c2 11 d2 12 e2 13 f2 14 g2 15 h2
//!    0 a1  1 b1  2 c1  3 d1  4 e1  5 f1  6 g1  7 h1  ← rank 0
//!    file 0         file 4         file 7
//! ```
//!
//! `a1 = 0` means `1u64 << sq` is the bit for that square in a `Bitboard`.
//! This matches Stockfish and most engines (see chessprogramming.org
//! “Square Mapping Considerations”).
//!
//! # Bitboards
//!
//! A `Bitboard` is a `u64` where bit `n` corresponds to `Square(n)`. This
//! lets us represent any set of squares (e.g. “all white pawns”) as a single
//! integer and use bitwise ops for blazing-fast queries. See
//! chessprogramming.org “Bitboards”.

use std::fmt;
use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not};

// ---------------------------------------------------------------------------
// Square
// ---------------------------------------------------------------------------

/// A board square 0..63, `a1 = 0` .. `h8 = 63`.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Square(pub u8);

impl Square {
    /// Create without bounds check — caller must guarantee `0..64`.
    #[inline]
    pub const fn new(index: u8) -> Self {
        debug_assert!(index < 64);
        Self(index)
    }

    /// Create from file (0..7) and rank (0..7).
    #[inline]
    pub const fn from_coords(file: u8, rank: u8) -> Self {
        debug_assert!(file < 8 && rank < 8);
        Self(rank * 8 + file)
    }

    /// Raw index `0..64`.
    #[inline]
    pub const fn index(self) -> u8 {
        self.0
    }

    /// File `0..7` (`a=0, h=7`).
    #[inline]
    pub const fn file(self) -> u8 {
        self.0 % 8
    }

    /// Rank `0..7` (`1=0, 8=7`).
    #[inline]
    pub const fn rank(self) -> u8 {
        self.0 / 8
    }

    /// Parse `"e4"` → `Some(Square)`, case-insensitive for file.
    pub fn from_str(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if bytes.len() != 2 {
            return None;
        }
        let file = match bytes[0] {
            b'a'..=b'h' => bytes[0] - b'a',
            b'A'..=b'H' => bytes[0] - b'A',
            _ => return None,
        };
        let rank = match bytes[1] {
            b'1'..=b'8' => bytes[1] - b'1',
            _ => return None,
        };
        Some(Self::from_coords(file, rank))
    }

    /// All 64 squares in LEFR order.
    pub const ALL: [Square; 64] = {
        let mut arr = [Square(0); 64];
        let mut i = 0;
        while i < 64 {
            arr[i] = Square(i as u8);
            i += 1;
        }
        arr
    };
}

impl fmt::Display for Square {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let file = (b'a' + self.file()) as char;
        let rank = (b'1' + self.rank()) as char;
        write!(f, "{file}{rank}")
    }
}

// Named constants — handy for tests and for `Position::startpos()`.
pub const A1: Square = Square(0);
pub const B1: Square = Square(1);
pub const C1: Square = Square(2);
pub const D1: Square = Square(3);
pub const E1: Square = Square(4);
pub const F1: Square = Square(5);
pub const G1: Square = Square(6);
pub const H1: Square = Square(7);
pub const A8: Square = Square(56);
pub const C8: Square = Square(58);
pub const D8: Square = Square(59);
pub const E8: Square = Square(60);
pub const F8: Square = Square(61);
pub const G8: Square = Square(62);
pub const H8: Square = Square(63);
pub const E4: Square = Square(28);

// ---------------------------------------------------------------------------
// Color
// ---------------------------------------------------------------------------

/// Side to move.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum Color {
    White = 0,
    Black = 1,
}

impl Color {
    #[inline]
    pub const fn opposite(self) -> Self {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }

    #[inline]
    pub const fn as_usize(self) -> usize {
        self as usize
    }

    #[inline]
    pub const fn is_white(self) -> bool {
        matches!(self, Color::White)
    }
}

// ---------------------------------------------------------------------------
// PieceType
// ---------------------------------------------------------------------------

/// Uncolored piece kind.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum PieceType {
    Pawn = 0,
    Knight = 1,
    Bishop = 2,
    Rook = 3,
    Queen = 4,
    King = 5,
}

impl PieceType {
    pub const NUM: usize = 6;
    pub const ALL: [PieceType; 6] = [
        PieceType::Pawn,
        PieceType::Knight,
        PieceType::Bishop,
        PieceType::Rook,
        PieceType::Queen,
        PieceType::King,
    ];

    /// `p` → `Pawn`, `n` → `Knight`, etc. Case-insensitive.
    pub fn from_char(c: char) -> Option<Self> {
        match c.to_ascii_lowercase() {
            'p' => Some(PieceType::Pawn),
            'n' => Some(PieceType::Knight),
            'b' => Some(PieceType::Bishop),
            'r' => Some(PieceType::Rook),
            'q' => Some(PieceType::Queen),
            'k' => Some(PieceType::King),
            _ => None,
        }
    }

    /// Uncolored char: `Pawn` → `'p'`.
    pub const fn to_char(self) -> char {
        match self {
            PieceType::Pawn => 'p',
            PieceType::Knight => 'n',
            PieceType::Bishop => 'b',
            PieceType::Rook => 'r',
            PieceType::Queen => 'q',
            PieceType::King => 'k',
        }
    }
}

// ---------------------------------------------------------------------------
// Piece
// ---------------------------------------------------------------------------

/// Colored piece `0..12` encoded as `color*6 + piece_type`.
///
/// Ordering: `WP=0, WN=1, WB=2, WR=3, WQ=4, WK=5, BP=6, BN=7, BB=8, BR=9, BQ=10, BK=11`
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub struct Piece(pub u8);

impl Piece {
    pub const WP: Piece = Piece(0);
    pub const WN: Piece = Piece(1);
    pub const WB: Piece = Piece(2);
    pub const WR: Piece = Piece(3);
    pub const WQ: Piece = Piece(4);
    pub const WK: Piece = Piece(5);
    pub const BP: Piece = Piece(6);
    pub const BN: Piece = Piece(7);
    pub const BB: Piece = Piece(8);
    pub const BR: Piece = Piece(9);
    pub const BQ: Piece = Piece(10);
    pub const BK: Piece = Piece(11);

    pub const ALL: [Piece; 12] = [
        Piece::WP,
        Piece::WN,
        Piece::WB,
        Piece::WR,
        Piece::WQ,
        Piece::WK,
        Piece::BP,
        Piece::BN,
        Piece::BB,
        Piece::BR,
        Piece::BQ,
        Piece::BK,
    ];

    #[inline]
    pub const fn new(color: Color, ty: PieceType) -> Self {
        Self(color as u8 * 6 + ty as u8)
    }

    #[inline]
    pub const fn color(self) -> Color {
        if self.0 < 6 {
            Color::White
        } else {
            Color::Black
        }
    }

    #[inline]
    pub const fn piece_type(self) -> PieceType {
        match self.0 % 6 {
            0 => PieceType::Pawn,
            1 => PieceType::Knight,
            2 => PieceType::Bishop,
            3 => PieceType::Rook,
            4 => PieceType::Queen,
            _ => PieceType::King,
        }
    }

    /// `P` → `WP`, `k` → `BK`. Case encodes color.
    pub fn from_char(c: char) -> Option<Self> {
        let ty = PieceType::from_char(c)?;
        let color = if c.is_ascii_uppercase() {
            Color::White
        } else {
            Color::Black
        };
        Some(Self::new(color, ty))
    }

    /// Colored char: `WP` → `'P'`, `BK` → `'k'`.
    pub fn to_char(self) -> char {
        let base = self.piece_type().to_char();
        match self.color() {
            Color::White => base.to_ascii_uppercase(),
            Color::Black => base,
        }
    }

    /// Index `0..12` for array lookups.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

// ---------------------------------------------------------------------------
// Bitboard
// ---------------------------------------------------------------------------

/// Set of squares as a `u64` — bit `n` is `Square(n)`.
///
/// The workhorse of a bitboard engine: a single `u64` can represent “all
/// white pawns” and operations like `pawns & file_bb(3)` are one CPU
/// instruction. See chessprogramming.org “Bitboards”.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Bitboard(pub u64);

impl Bitboard {
    pub const EMPTY: Bitboard = Bitboard(0);
    pub const ALL: Bitboard = Bitboard(!0);

    #[inline]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[inline]
    pub const fn from_u64(bits: u64) -> Self {
        Self(bits)
    }

    #[inline]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    #[inline]
    pub const fn from_sq(sq: Square) -> Self {
        Self(1u64 << sq.0)
    }

    #[inline]
    pub fn contains(self, sq: Square) -> bool {
        (self.0 >> sq.0) & 1 == 1
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub fn count(self) -> u32 {
        self.0.count_ones()
    }

    /// Least-significant 1-bit as `Square`, if any.
    #[inline]
    pub fn lsb(self) -> Option<Square> {
        if self.0 == 0 {
            None
        } else {
            Some(Square(self.0.trailing_zeros() as u8))
        }
    }

    /// Remove and return the LSB, if any.
    #[inline]
    pub fn pop_lsb(&mut self) -> Option<Square> {
        let sq = self.lsb()?;
        self.0 &= self.0 - 1; // clear LSB
        Some(sq)
    }

    /// Iterate over set squares LSB-first.
    pub fn squares(self) -> BitboardIter {
        BitboardIter(self)
    }

    /// File mask: all squares on `file` 0..7.
    pub fn file_bb(file: u8) -> Self {
        debug_assert!(file < 8);
        Self(0x0101010101010101u64 << file)
    }

    /// Rank mask: all squares on `rank` 0..7.
    pub fn rank_bb(rank: u8) -> Self {
        debug_assert!(rank < 8);
        Self(0xFFu64 << (rank * 8))
    }
}

/// Iterator over `Bitboard` squares.
pub struct BitboardIter(pub Bitboard);

impl Iterator for BitboardIter {
    type Item = Square;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.pop_lsb()
    }
}

// Bitwise ops — so `a | b`, `a & b`, `!a` work naturally.
impl BitOr for Bitboard {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl BitOrAssign for Bitboard {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}
impl BitAnd for Bitboard {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl BitAndAssign for Bitboard {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}
impl BitXor for Bitboard {
    type Output = Self;
    fn bitxor(self, rhs: Self) -> Self {
        Self(self.0 ^ rhs.0)
    }
}
impl BitXorAssign for Bitboard {
    fn bitxor_assign(&mut self, rhs: Self) {
        self.0 ^= rhs.0;
    }
}
impl Not for Bitboard {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_coords() {
        assert_eq!(A1.file(), 0);
        assert_eq!(A1.rank(), 0);
        assert_eq!(H8.file(), 7);
        assert_eq!(H8.rank(), 7);
        assert_eq!(Square::from_coords(4, 3), Square(28)); // e4
        assert_eq!(E4.to_string(), "e4");
        assert_eq!(Square::from_str("e4"), Some(E4));
        assert_eq!(Square::from_str("E4"), Some(E4));
        assert_eq!(Square::from_str("i9"), None);
        assert_eq!(Square::from_str(""), None);
    }

    #[test]
    fn piece_color_type() {
        assert_eq!(Piece::WP.color(), Color::White);
        assert_eq!(Piece::BK.color(), Color::Black);
        assert_eq!(Piece::WP.piece_type(), PieceType::Pawn);
        assert_eq!(Piece::BK.piece_type(), PieceType::King);
        assert_eq!(Piece::from_char('P'), Some(Piece::WP));
        assert_eq!(Piece::from_char('k'), Some(Piece::BK));
        assert_eq!(Piece::from_char('x'), None);
        assert_eq!(Piece::WP.to_char(), 'P');
        assert_eq!(Piece::BK.to_char(), 'k');
    }

    #[test]
    fn bitboard_ops() {
        let mut bb = Bitboard::from_sq(A1) | Bitboard::from_sq(H8);
        assert_eq!(bb.count(), 2);
        assert!(bb.contains(A1));
        assert!(bb.contains(H8));
        assert!(!bb.contains(E4));
        assert_eq!(bb.lsb(), Some(A1));
        assert_eq!(bb.pop_lsb(), Some(A1));
        assert_eq!(bb.pop_lsb(), Some(H8));
        assert!(bb.is_empty());
        let mut empty = Bitboard::EMPTY;
        assert_eq!(empty.pop_lsb(), None);
    }

    #[test]
    fn bitboard_files_ranks() {
        assert_eq!(Bitboard::file_bb(0).count(), 8);
        assert_eq!(Bitboard::rank_bb(0).count(), 8);
        assert!(Bitboard::file_bb(0).contains(A1));
        assert!(Bitboard::file_bb(0).contains(A8));
        assert!(!Bitboard::file_bb(0).contains(B1));
    }
}

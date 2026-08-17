#![allow(unused)]
//! Move representation — a struct with from/to/promotion.
//!
//! We use a `struct` (not a packed `u16`) for readability — `Move { from, to, promotion }`
//! is self-documenting and `Copy` cheap. A `u16` would save bytes but obscure
//! the learning. Promotion is `Option<PieceType>` (None = not a promotion;
//! Some(Q/R/B/N) = promote to that piece). Castling and en passant are
//! inferred from `from`/`to` + `Position` during `make_move` (king moves two
//! squares = castle; pawn to en-passant square = en passant).

use crate::board::types::{PieceType, Square};

/// A move from `from` to `to`, optionally promoting.
///
/// `Copy` so perft can pass moves by value without cloning.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Move {
    pub from: Square,
    pub to: Square,
    pub promotion: Option<PieceType>,
}

impl Move {
    /// Create a move. `promotion` is `Some` only for pawn promotions.
    #[inline]
    pub const fn new(from: Square, to: Square, promotion: Option<PieceType>) -> Self {
        Self {
            from,
            to,
            promotion,
        }
    }

    /// Quiet move (no promotion).
    #[inline]
    pub const fn quiet(from: Square, to: Square) -> Self {
        Self {
            from,
            to,
            promotion: None,
        }
    }

    /// Promotion move.
    #[inline]
    pub const fn promotion(from: Square, to: Square, promo: PieceType) -> Self {
        Self {
            from,
            to,
            promotion: Some(promo),
        }
    }

    /// Parse UCI string like `"e2e4"` or `"e7e8q"` (promotion char is `q/r/b/n`).
    /// Returns `None` if format is bad; legality is checked later by `Position::make_move`.
    pub fn parse_uci(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if bytes.len() < 4 || bytes.len() > 5 {
            return None;
        }
        let from = Square::from_str(&s[0..2])?;
        let to = Square::from_str(&s[2..4])?;
        let promotion = if bytes.len() == 5 {
            let p = match bytes[4] {
                b'q' | b'Q' => PieceType::Queen,
                b'r' | b'R' => PieceType::Rook,
                b'b' | b'B' => PieceType::Bishop,
                b'n' | b'N' => PieceType::Knight,
                _ => return None,
            };
            Some(p)
        } else {
            None
        };
        Some(Self {
            from,
            to,
            promotion,
        })
    }

    /// True if this move is a promotion.
    #[inline]
    pub fn is_promotion(self) -> bool {
        self.promotion.is_some()
    }
}

impl std::fmt::Display for Move {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.from, self.to)?;
        if let Some(p) = self.promotion {
            let c = match p {
                PieceType::Queen => 'q',
                PieceType::Rook => 'r',
                PieceType::Bishop => 'b',
                PieceType::Knight => 'n',
                _ => 'q',
            };
            write!(f, "{c}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::types::Square;

    #[test]
    fn parse_and_display() {
        let m = Move::parse_uci("e2e4").unwrap();
        assert_eq!(m.from, Square::from_str("e2").unwrap());
        assert_eq!(m.to, Square::from_str("e4").unwrap());
        assert_eq!(m.promotion, None);
        assert_eq!(m.to_string(), "e2e4");

        let m = Move::parse_uci("e7e8q").unwrap();
        assert_eq!(m.promotion, Some(PieceType::Queen));
        assert_eq!(m.to_string(), "e7e8q");

        assert!(Move::parse_uci("e2e").is_none());
        assert!(Move::parse_uci("e9e2").is_none());
        assert!(Move::parse_uci("e2e4x").is_none());
        assert!(Move::parse_uci("").is_none());
    }

    #[test]
    fn quiet_and_promotion_const() {
        let from = Square::from_str("a2").unwrap();
        let to = Square::from_str("a1").unwrap();
        let m = Move::quiet(from, to);
        assert_eq!(m.promotion, None);
        let m2 = Move::promotion(from, to, PieceType::Queen);
        assert_eq!(m2.promotion, Some(PieceType::Queen));
    }
}

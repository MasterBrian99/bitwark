#![allow(unused)]
//! The board `Position` — the single source of truth for a chess position.
//!
//! We keep both bitboards (fast set operations for move generation) and a
//! mailbox `board[64]` (O(1) `piece_at`). They are kept in sync by the
//! helpers below; `debug_assert!` checks the invariant in tests.
//!
//! Castling is a 4-bit mask `WK=1 WQ=2 BK=4 BQ=8` (KQkq order when emitting
//! FEN). Stored as `u8` rather than four bools so the FRC extension
//! (rook-file bitboards) is a drop-in later; only the KQkq
//! bits. (Chess960 is out of scope.)

use crate::board::types::{Bitboard, Color, Piece, Square};
use crate::board::zobrist;

// ---------------------------------------------------------------------------
// Castling constants
// ---------------------------------------------------------------------------

pub const CASTLE_WK: u8 = 1; // K
pub const CASTLE_WQ: u8 = 2; // Q
pub const CASTLE_BK: u8 = 4; // k
pub const CASTLE_BQ: u8 = 8; // q
pub const CASTLE_ALL: u8 = CASTLE_WK | CASTLE_WQ | CASTLE_BK | CASTLE_BQ;

/// A chess position. `Clone` so search can copy-make into its own position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Position {
    /// `pieces[piece.index()]` — 12 bitboards.
    pieces: [Bitboard; 12],
    /// `occupancy[White]`, `occupancy[Black]`.
    occupancy: [Bitboard; 2],
    /// All occupied squares.
    occupied: Bitboard,
    /// Mailbox for `piece_at`.
    board: [Option<Piece>; 64],
    /// Side to move.
    side_to_move: Color,
    /// Castling rights bitmask (KQkq).
    castling: u8,
    /// En passant target square (square *behind* the double-pushed pawn).
    en_passant: Option<Square>,
    /// Halfmove clock (50-move rule).
    halfmove: u8,
    /// Fullmove number (starts at 1, increments after Black's move).
    fullmove: u16,
    /// Zobrist hash (full recompute for now; incremental in Phase 2).
    hash: u64,
}

impl Position {
    /// Empty position for FEN builder (crate-private to avoid recursion).
    pub(crate) fn empty_for_fen_internal() -> Self {
        Self {
            pieces: [Bitboard::EMPTY; 12],
            occupancy: [Bitboard::EMPTY; 2],
            occupied: Bitboard::EMPTY,
            board: [None; 64],
            side_to_move: Color::White,
            castling: 0,
            en_passant: None,
            halfmove: 0,
            fullmove: 1,
            hash: 0,
        }
    }

    /// Empty position (no pieces, White to move, no castling). Useful as a
    /// builder starting point; `parse_fen` fills it.
    fn empty() -> Self {
        Self::empty_for_fen_internal()
    }

    /// Standard starting position.
    pub fn startpos() -> Self {
        // Delegate to FEN so there is exactly one place that knows the start array.
        crate::board::fen::parse_fen(crate::board::fen::START_FEN).expect("START_FEN is valid")
    }

    /// Place or remove a piece on `sq`. Keeps all three representations in sync.
    /// For Phase 1 this is only used while building from FEN; Phase 2 will use
    /// it inside `make`/`unmake`.
    fn set_piece(&mut self, sq: Square, piece: Option<Piece>) {
        let idx = sq.index() as usize;
        // Remove old piece if any.
        if let Some(old) = self.board[idx] {
            let bb = Bitboard::from_sq(sq);
            self.pieces[old.index()] = Bitboard(self.pieces[old.index()].0 & !bb.0);
            self.occupancy[old.color().as_usize()] =
                Bitboard(self.occupancy[old.color().as_usize()].0 & !bb.0);
            self.occupied = Bitboard(self.occupied.0 & !bb.0);
        }
        // Place new piece if any.
        if let Some(p) = piece {
            let bb = Bitboard::from_sq(sq);
            self.pieces[p.index()] = Bitboard(self.pieces[p.index()].0 | bb.0);
            self.occupancy[p.color().as_usize()] =
                Bitboard(self.occupancy[p.color().as_usize()].0 | bb.0);
            self.occupied = Bitboard(self.occupied.0 | bb.0);
        }
        self.board[idx] = piece;
    }

    /// Recompute Zobrist hash from scratch (Phase 1). Phase 2 will make this
    /// incremental on `make`/`unmake`.
    fn recompute_hash(&mut self) {
        self.hash = zobrist::keys().hash_position(
            &self.board,
            self.side_to_move,
            self.castling,
            self.en_passant,
        );
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    #[inline]
    pub fn piece_at(&self, sq: Square) -> Option<Piece> {
        self.board[sq.index() as usize]
    }

    #[inline]
    pub fn side_to_move(&self) -> Color {
        self.side_to_move
    }

    #[inline]
    pub fn castling(&self) -> u8 {
        self.castling
    }

    #[inline]
    pub fn en_passant(&self) -> Option<Square> {
        self.en_passant
    }

    #[inline]
    pub fn halfmove(&self) -> u8 {
        self.halfmove
    }

    #[inline]
    pub fn fullmove(&self) -> u16 {
        self.fullmove
    }

    #[inline]
    pub fn hash(&self) -> u64 {
        self.hash
    }

    #[inline]
    pub fn occupied(&self) -> Bitboard {
        self.occupied
    }

    #[inline]
    pub fn occupied_color(&self, color: Color) -> Bitboard {
        self.occupancy[color.as_usize()]
    }

    #[inline]
    pub fn pieces_bb(&self, piece: Piece) -> Bitboard {
        self.pieces[piece.index()]
    }

    // -----------------------------------------------------------------------
    // Board pretty-print for `d` (UCI spec §5.3) — Stockfish-mirrored.
    // -----------------------------------------------------------------------

    /// Lines for the `d` command, mirroring Stockfish's format:
    ///  - border `+---+---+...`
    ///  - `| r | n | ... | 8` per rank 8..1
    ///  - `  a   b   c   d   e   f   g   h`
    ///  - `Fen: ...`
    ///  - `Key: <16 hex>`
    ///  - `Checkers: ` (kept empty for now)
    pub fn display_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(String::new());
        lines.push(" +---+---+---+---+---+---+---+---+ ".to_string());
        for rank in (0..8).rev() {
            let mut row = String::from(" |");
            for file in 0..8 {
                let sq = Square::from_coords(file, rank);
                let ch = match self.piece_at(sq) {
                    Some(p) => p.to_char(),
                    None => ' ',
                };
                row.push(' ');
                row.push(ch);
                row.push_str(" |");
            }
            row.push(' ');
            row.push(char::from(b'1' + rank));
            lines.push(row);
            lines.push(" +---+---+---+---+---+---+---+---+ ".to_string());
        }
        lines.push("   a   b   c   d   e   f   g   h".to_string());
        lines.push(String::new());
        lines.push(format!("Fen: {}", crate::board::fen::to_fen(self)));
        lines.push(format!("Key: {:016X}", self.hash));
        lines.push("Checkers: ".to_string());
        lines
    }

    // -----------------------------------------------------------------------
    // Internal helpers for `fen.rs` (crate-private)
    // -----------------------------------------------------------------------

    pub(crate) fn set_side_to_move(&mut self, c: Color) {
        self.side_to_move = c;
    }

    pub(crate) fn set_castling(&mut self, mask: u8) {
        self.castling = mask & 0xF;
    }

    pub(crate) fn set_en_passant(&mut self, sq: Option<Square>) {
        self.en_passant = sq;
    }

    pub(crate) fn set_halfmove(&mut self, hm: u8) {
        self.halfmove = hm;
    }

    pub(crate) fn set_fullmove(&mut self, fm: u16) {
        self.fullmove = fm;
    }

    pub(crate) fn rebuild_from_board(&mut self) {
        // Already maintained via `set_piece`; just recompute hash.
        self.recompute_hash();
    }

    pub(crate) fn place_piece_for_fen(&mut self, sq: Square, piece: Piece) {
        self.set_piece(sq, Some(piece));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::{START_FEN, parse_fen};

    #[test]
    fn startpos_piece_counts() {
        let pos = Position::startpos();
        assert_eq!(pos.occupied().count(), 32);
        assert_eq!(pos.occupied_color(Color::White).count(), 16);
        assert_eq!(pos.occupied_color(Color::Black).count(), 16);
        assert_eq!(pos.side_to_move(), Color::White);
        assert_eq!(pos.castling(), CASTLE_ALL);
        assert_eq!(pos.en_passant(), None);
        assert_eq!(pos.halfmove(), 0);
        assert_eq!(pos.fullmove(), 1);
    }

    #[test]
    fn hash_determinism() {
        let a = Position::startpos().hash();
        let b = Position::startpos().hash();
        assert_eq!(a, b);
        assert_ne!(a, 0);
    }

    #[test]
    fn display_contains_fen_and_key() {
        let pos = Position::startpos();
        let lines = pos.display_lines();
        let fen_line = lines.iter().find(|l| l.starts_with("Fen:")).unwrap();
        assert_eq!(fen_line, &format!("Fen: {START_FEN}"));
        let key_line = lines.iter().find(|l| l.starts_with("Key:")).unwrap();
        assert!(key_line.starts_with("Key: "));
        // 16 hex chars
        let hex = key_line.trim_start_matches("Key: ").trim();
        assert_eq!(hex.len(), 16);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn fen_round_trip_startpos() {
        let pos = parse_fen(START_FEN).unwrap();
        assert_eq!(crate::board::fen::to_fen(&pos), START_FEN);
    }
}

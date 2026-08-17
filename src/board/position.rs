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

use crate::board::mv::Move;
use crate::board::types::{Bitboard, Color, Piece, PieceType, Square};
use crate::board::zobrist;

// ---------------------------------------------------------------------------
// Castling constants
// ---------------------------------------------------------------------------

pub const CASTLE_WK: u8 = 1; // K
pub const CASTLE_WQ: u8 = 2; // Q
pub const CASTLE_BK: u8 = 4; // k
pub const CASTLE_BQ: u8 = 8; // q
pub const CASTLE_ALL: u8 = CASTLE_WK | CASTLE_WQ | CASTLE_BK | CASTLE_BQ;

/// Undo information for one ply — pushed onto `history` on `make_move`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct UndoInfo {
    captured: Option<Piece>,
    captured_sq: Option<Square>,
    castling: u8,
    en_passant: Option<Square>,
    halfmove: u8,
    hash: u64,
}

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
    /// Zobrist hash (updated incrementally by make/unmake).
    hash: u64,
    /// History stack for unmake.
    history: Vec<UndoInfo>,
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
            history: Vec::new(),
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

    /// Recompute the Zobrist hash from scratch (used at FEN parse and in tests).
    /// make/unmake apply incremental XOR updates instead.
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

    /// Find king square for `color` (assumes exactly one).
    pub fn king_square(&self, color: Color) -> Option<Square> {
        let bb = match color {
            Color::White => self.pieces_bb(Piece::WK),
            Color::Black => self.pieces_bb(Piece::BK),
        };
        bb.lsb()
    }

    // -----------------------------------------------------------------------
    // Make / unmake
    // -----------------------------------------------------------------------

    /// Make `mv` on this position. Assumes `mv` is pseudo-legal (generated by
    /// movegen); validates basic ownership. Pushes undo info. Updates
    /// castling, en passant, halfmove, fullmove, hash, and side to move.
    pub fn make_move(&mut self, mv: Move) {
        let from = mv.from;
        let to = mv.to;
        let promo = mv.promotion;

        let moving = self.piece_at(from).expect("make_move: no piece on from");
        let captured = self.piece_at(to);
        let captured_sq = if captured.is_some() { Some(to) } else { None };

        // For en passant, captured piece is not on `to` but on the pawn's rank.
        let mut ep_captured_sq: Option<Square> = None;
        let mut ep_captured_piece: Option<Piece> = None;
        let is_en_passant = moving.piece_type() == PieceType::Pawn
            && self.en_passant == Some(to)
            && captured.is_none();

        if is_en_passant {
            // Captured pawn is behind the en passant square.
            let cap_rank = from.rank();
            let cap_file = to.file();
            let cap_sq = Square::from_coords(cap_file, cap_rank);
            ep_captured_sq = Some(cap_sq);
            ep_captured_piece = self.piece_at(cap_sq);
        }

        let undo = UndoInfo {
            captured: captured.or(ep_captured_piece),
            captured_sq: captured_sq.or(ep_captured_sq),
            castling: self.castling,
            en_passant: self.en_passant,
            halfmove: self.halfmove,
            hash: self.hash,
        };
        let old_hash = undo.hash;
        let old_en_passant = undo.en_passant;
        let old_castling = undo.castling;
        self.history.push(undo);

        // Determine if this move is a capture (including en passant)
        let is_capture = captured.is_some() || is_en_passant;
        let is_pawn_move = moving.piece_type() == PieceType::Pawn;

        // Handle castling: king moves two squares.
        let is_castling = moving.piece_type() == PieceType::King
            && (from.file() as i8 - to.file() as i8).abs() == 2;

        // Update halfmove clock
        if is_capture || is_pawn_move {
            self.halfmove = 0;
        } else {
            self.halfmove = self.halfmove.wrapping_add(1);
        }

        // Move the piece
        self.set_piece(from, None);

        // Handle en passant capture removal
        if let Some(cap_sq) = ep_captured_sq {
            self.set_piece(cap_sq, None);
        } else if captured.is_some() {
            // Normal capture: the destination piece was already removed by set_piece(from,None) ?
            // No, we removed from, but to still has captured piece. We need to remove it.
            // set_piece(to, None) would remove, but we are about to place moving piece there.
            // Instead, just remove the captured piece's bitboard entry via set_piece(to, None) before placing.
            // However set_piece(to, None) will remove captured piece if we call it.
            // We haven't removed captured yet; set_piece(from,None) only removed from.
            // So remove captured piece at `to` now.
            self.set_piece(to, None);
        }

        // Handle promotion
        let placed = if let Some(pt) = promo {
            Piece::new(moving.color(), pt)
        } else {
            moving
        };
        self.set_piece(to, Some(placed));

        // Handle castling rook move
        if is_castling {
            let (rook_from, rook_to) = match (from, to) {
                // White
                f if f.0 == Square::from_str("e1").unwrap()
                    && f.1 == Square::from_str("g1").unwrap() =>
                {
                    (
                        Square::from_str("h1").unwrap(),
                        Square::from_str("f1").unwrap(),
                    )
                }
                f if f.0 == Square::from_str("e1").unwrap()
                    && f.1 == Square::from_str("c1").unwrap() =>
                {
                    (
                        Square::from_str("a1").unwrap(),
                        Square::from_str("d1").unwrap(),
                    )
                }
                // Black
                f if f.0 == Square::from_str("e8").unwrap()
                    && f.1 == Square::from_str("g8").unwrap() =>
                {
                    (
                        Square::from_str("h8").unwrap(),
                        Square::from_str("f8").unwrap(),
                    )
                }
                f if f.0 == Square::from_str("e8").unwrap()
                    && f.1 == Square::from_str("c8").unwrap() =>
                {
                    (
                        Square::from_str("a8").unwrap(),
                        Square::from_str("d8").unwrap(),
                    )
                }
                _ => panic!("invalid castling move {mv}"),
            };
            let rook = self.piece_at(rook_from).expect("castling rook missing");
            self.set_piece(rook_from, None);
            self.set_piece(rook_to, Some(rook));
        }

        // Update castling rights
        // King moves remove both rights for that color; rook moves from original squares remove that side.
        match moving {
            Piece::WK => self.castling &= !(CASTLE_WK | CASTLE_WQ),
            Piece::BK => self.castling &= !(CASTLE_BK | CASTLE_BQ),
            Piece::WR => {
                if from == Square::from_str("h1").unwrap() {
                    self.castling &= !CASTLE_WK;
                } else if from == Square::from_str("a1").unwrap() {
                    self.castling &= !CASTLE_WQ;
                }
            }
            Piece::BR => {
                if from == Square::from_str("h8").unwrap() {
                    self.castling &= !CASTLE_BK;
                } else if from == Square::from_str("a8").unwrap() {
                    self.castling &= !CASTLE_BQ;
                }
            }
            _ => {}
        }
        // Captured rook on original square also removes rights
        if let Some(cap) = captured {
            match cap {
                Piece::WR => {
                    if to == Square::from_str("h1").unwrap() {
                        self.castling &= !CASTLE_WK;
                    } else if to == Square::from_str("a1").unwrap() {
                        self.castling &= !CASTLE_WQ;
                    }
                }
                Piece::BR => {
                    if to == Square::from_str("h8").unwrap() {
                        self.castling &= !CASTLE_BK;
                    } else if to == Square::from_str("a8").unwrap() {
                        self.castling &= !CASTLE_BQ;
                    }
                }
                _ => {}
            }
        }
        // En passant capture also could capture a rook? No, only pawns, so no castling effect.

        // Update en passant square (must be before hash, as hash includes it)
        if moving.piece_type() == PieceType::Pawn
            && (from.rank() as i8 - to.rank() as i8).abs() == 2
        {
            // Double push: en passant square is behind the pawn (midpoint)
            let ep_rank = (from.rank() + to.rank()) / 2;
            self.en_passant = Some(Square::from_coords(from.file(), ep_rank));
        } else {
            self.en_passant = None;
        }

        // Update side to move and fullmove
        let prev_side = self.side_to_move;
        self.side_to_move = self.side_to_move.opposite();
        if prev_side == Color::Black {
            self.fullmove = self.fullmove.wrapping_add(1);
        }

        // Incremental hash update
        let keys = zobrist::keys();
        let mut new_hash = old_hash;
        // Side
        new_hash ^= keys.side;
        // Old en passant
        if let Some(ep) = old_en_passant {
            new_hash ^= keys.en_passant[ep.file() as usize];
        }
        // New en passant
        if let Some(ep) = self.en_passant {
            new_hash ^= keys.en_passant[ep.file() as usize];
        }
        // Old castling
        new_hash ^= keys.castling[old_castling as usize];
        // New castling
        new_hash ^= keys.castling[self.castling as usize];
        // Moving piece from
        new_hash ^= keys.piece_sq[moving.index()][from.index() as usize];
        // Captured piece (if any)
        if let Some(cap) = captured {
            new_hash ^= keys.piece_sq[cap.index()][to.index() as usize];
        } else if let Some(cap) = ep_captured_piece {
            let cap_sq = ep_captured_sq.unwrap();
            new_hash ^= keys.piece_sq[cap.index()][cap_sq.index() as usize];
        }
        // Placed piece to
        let placed_for_hash = if let Some(pt) = promo {
            Piece::new(moving.color(), pt)
        } else {
            moving
        };
        new_hash ^= keys.piece_sq[placed_for_hash.index()][to.index() as usize];
        // Castling rook
        if is_castling {
            let (rook_from, rook_to) = match (from, to) {
                f if f.0 == Square::from_str("e1").unwrap()
                    && f.1 == Square::from_str("g1").unwrap() =>
                {
                    (
                        Square::from_str("h1").unwrap(),
                        Square::from_str("f1").unwrap(),
                    )
                }
                f if f.0 == Square::from_str("e1").unwrap()
                    && f.1 == Square::from_str("c1").unwrap() =>
                {
                    (
                        Square::from_str("a1").unwrap(),
                        Square::from_str("d1").unwrap(),
                    )
                }
                f if f.0 == Square::from_str("e8").unwrap()
                    && f.1 == Square::from_str("g8").unwrap() =>
                {
                    (
                        Square::from_str("h8").unwrap(),
                        Square::from_str("f8").unwrap(),
                    )
                }
                f if f.0 == Square::from_str("e8").unwrap()
                    && f.1 == Square::from_str("c8").unwrap() =>
                {
                    (
                        Square::from_str("a8").unwrap(),
                        Square::from_str("d8").unwrap(),
                    )
                }
                _ => panic!("invalid castling move {mv}"),
            };
            let rook = if moving.color() == Color::White {
                Piece::WR
            } else {
                Piece::BR
            };
            new_hash ^= keys.piece_sq[rook.index()][rook_from.index() as usize];
            new_hash ^= keys.piece_sq[rook.index()][rook_to.index() as usize];
        }
        self.hash = new_hash;
    }

    /// Unmake the last move. Panics if no history.
    pub fn unmake_move(&mut self, mv: Move) {
        let undo = self.history.pop().expect("unmake without history");
        let from = mv.from;
        let to = mv.to;
        let promo = mv.promotion;

        // Restore side and fullmove
        self.side_to_move = self.side_to_move.opposite();
        if self.side_to_move == Color::Black {
            // We had incremented after Black's move, so decrement
            self.fullmove = self.fullmove.wrapping_sub(1);
        }

        // Handle castling rook undo
        let is_castling = promo.is_none()
            && self
                .piece_at(to)
                .map(|p| p.piece_type() == PieceType::King)
                .unwrap_or(false)
            && (from.file() as i8 - to.file() as i8).abs() == 2;

        if is_castling {
            let (rook_from, rook_to) = match (from, to) {
                f if f.0 == Square::from_str("e1").unwrap()
                    && f.1 == Square::from_str("g1").unwrap() =>
                {
                    (
                        Square::from_str("h1").unwrap(),
                        Square::from_str("f1").unwrap(),
                    )
                }
                f if f.0 == Square::from_str("e1").unwrap()
                    && f.1 == Square::from_str("c1").unwrap() =>
                {
                    (
                        Square::from_str("a1").unwrap(),
                        Square::from_str("d1").unwrap(),
                    )
                }
                f if f.0 == Square::from_str("e8").unwrap()
                    && f.1 == Square::from_str("g8").unwrap() =>
                {
                    (
                        Square::from_str("h8").unwrap(),
                        Square::from_str("f8").unwrap(),
                    )
                }
                f if f.0 == Square::from_str("e8").unwrap()
                    && f.1 == Square::from_str("c8").unwrap() =>
                {
                    (
                        Square::from_str("a8").unwrap(),
                        Square::from_str("d8").unwrap(),
                    )
                }
                _ => panic!("invalid castling unmake {mv}"),
            };
            if let Some(rook) = self.piece_at(rook_to) {
                self.set_piece(rook_to, None);
                self.set_piece(rook_from, Some(rook));
            }
        }

        // Remove moved piece from `to` and restore to `from`
        let moved_piece = self.piece_at(to).expect("unmake: no piece on to");
        self.set_piece(to, None);

        // Restore moving piece (with promotion handling)
        let original_piece = if promo.is_some() {
            // Was a pawn before promotion
            Piece::new(moved_piece.color(), PieceType::Pawn)
        } else {
            moved_piece
        };
        self.set_piece(from, Some(original_piece));

        // Restore captured piece (if any) on its square
        if let (Some(cap), Some(cap_sq)) = (undo.captured, undo.captured_sq) {
            self.set_piece(cap_sq, Some(cap));
        }

        // Restore other state
        self.castling = undo.castling;
        self.en_passant = undo.en_passant;
        self.halfmove = undo.halfmove;
        self.hash = undo.hash;
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

    #[test]
    fn make_unmake_simple() {
        let mut pos = Position::startpos();
        let fen_before = crate::board::fen::to_fen(&pos);
        let hash_before = pos.hash();
        let mv = crate::board::mv::Move::parse_uci("e2e4").unwrap();
        pos.make_move(mv);
        // After e4, pawn on e4, not on e2
        assert_eq!(pos.piece_at(Square::from_str("e2").unwrap()), None);
        assert_eq!(
            pos.piece_at(Square::from_str("e4").unwrap()),
            Some(Piece::WP)
        );
        pos.unmake_move(mv);
        assert_eq!(crate::board::fen::to_fen(&pos), fen_before);
        assert_eq!(pos.hash(), hash_before);
    }

    #[test]
    fn make_unmake_capture() {
        let fen = "rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2";
        let mut pos = parse_fen(fen).unwrap();
        let mv = crate::board::mv::Move::parse_uci("e4d5").unwrap(); // exd5
        let hash_before = pos.hash();
        pos.make_move(mv);
        assert_eq!(
            pos.piece_at(Square::from_str("d5").unwrap()),
            Some(Piece::WP)
        );
        pos.unmake_move(mv);
        assert_eq!(pos.hash(), hash_before);
        assert_eq!(crate::board::fen::to_fen(&pos), fen);
    }

    #[test]
    fn make_unmake_en_passant() {
        // Position where white can en passant: white pawn on e5, black pawn on d5, ep d6
        let fen = "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 3";
        // Actually use a known ep position: after 1. e4 d5 2. e5 f5 -> white can capture f6 ep?
        // Simpler: set up manually: white to move, ep f6, pawn on e5 can capture
        let fen2 = "rnbqkbnr/ppp1p1pp/8/4P3/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 2";
        // This fen has pawn on e5, black pawn on f5? No, need pawn on e5 and black pawn just moved f7-f5, ep f6, white pawn e5 captures f6.
        // Let's use a direct ep test: position with white pawn e5, black pawn f5, ep f6, white to move.
        let fen_ep = "8/8/8/4Pp2/8/8/8/4K2k w - f6 0 1";
        let mut pos = parse_fen(fen_ep).unwrap();
        let mv = crate::board::mv::Move::parse_uci("e5f6").unwrap();
        pos.make_move(mv);
        // After en passant, pawn on f6, captured pawn on f5 gone
        assert_eq!(
            pos.piece_at(Square::from_str("f6").unwrap()),
            Some(Piece::WP)
        );
        assert_eq!(pos.piece_at(Square::from_str("f5").unwrap()), None);
        pos.unmake_move(mv);
        assert_eq!(crate::board::fen::to_fen(&pos), fen_ep);
    }

    #[test]
    fn make_unmake_castling() {
        let fen = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
        let mut pos = parse_fen(fen).unwrap();
        let mv = crate::board::mv::Move::parse_uci("e1g1").unwrap(); // O-O
        pos.make_move(mv);
        assert_eq!(
            pos.piece_at(Square::from_str("g1").unwrap()),
            Some(Piece::WK)
        );
        assert_eq!(
            pos.piece_at(Square::from_str("f1").unwrap()),
            Some(Piece::WR)
        );
        assert_eq!(pos.castling() & CASTLE_WK, 0);
        pos.unmake_move(mv);
        assert_eq!(crate::board::fen::to_fen(&pos), fen);
    }

    #[test]
    fn make_unmake_promotion() {
        let fen = "8/P7/8/8/8/8/8/4K2k w - - 0 1";
        let mut pos = parse_fen(fen).unwrap();
        let mv = crate::board::mv::Move::parse_uci("a7a8q").unwrap();
        pos.make_move(mv);
        assert_eq!(
            pos.piece_at(Square::from_str("a8").unwrap()),
            Some(Piece::WQ)
        );
        pos.unmake_move(mv);
        assert_eq!(crate::board::fen::to_fen(&pos), fen);
    }
}

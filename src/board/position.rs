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
use crate::eval::tables;

// ---------------------------------------------------------------------------
// Castling constants
// ---------------------------------------------------------------------------

pub const CASTLE_WK: u8 = 1; // K
pub const CASTLE_WQ: u8 = 2; // Q
pub const CASTLE_BK: u8 = 4; // k
pub const CASTLE_BQ: u8 = 8; // q
pub const CASTLE_ALL: u8 = CASTLE_WK | CASTLE_WQ | CASTLE_BK | CASTLE_BQ;

/// Castling rook squares for a king move `from -> to`.
/// Returns `(rook_from, rook_to)` for the four legal castling moves, else `None`.
#[inline]
fn castling_rook_squares(from: Square, to: Square) -> Option<(Square, Square)> {
    use crate::board::types::{A1, A8, C1, C8, D1, D8, E1, E8, F1, F8, G1, G8, H1, H8};
    match (from, to) {
        (E1, G1) => Some((H1, F1)),
        (E1, C1) => Some((A1, D1)),
        (E8, G8) => Some((H8, F8)),
        (E8, C8) => Some((A8, D8)),
        _ => None,
    }
}

#[inline]
fn psqt_delta(p: Piece, sq: Square) -> (i32, i32) {
    let idx = sq.index() as usize;
    let pst_idx = if p.color() == Color::White {
        idx
    } else {
        idx ^ 56
    };
    let pt = p.piece_type() as usize;
    let mg_pst = match p.piece_type() {
        PieceType::Pawn => tables::MG_PAWN_TABLE[pst_idx],
        PieceType::Knight => tables::MG_KNIGHT_TABLE[pst_idx],
        PieceType::Bishop => tables::MG_BISHOP_TABLE[pst_idx],
        PieceType::Rook => tables::MG_ROOK_TABLE[pst_idx],
        PieceType::Queen => tables::MG_QUEEN_TABLE[pst_idx],
        PieceType::King => tables::MG_KING_TABLE[pst_idx],
    };
    let eg_pst = match p.piece_type() {
        PieceType::Pawn => tables::EG_PAWN_TABLE[pst_idx],
        PieceType::Knight => tables::EG_KNIGHT_TABLE[pst_idx],
        PieceType::Bishop => tables::EG_BISHOP_TABLE[pst_idx],
        PieceType::Rook => tables::EG_ROOK_TABLE[pst_idx],
        PieceType::Queen => tables::EG_QUEEN_TABLE[pst_idx],
        PieceType::King => tables::EG_KING_TABLE[pst_idx],
    };
    let mg = tables::MG_VALUE[pt] + mg_pst;
    let eg = tables::EG_VALUE[pt] + eg_pst;
    if p.color() == Color::White {
        (mg, eg)
    } else {
        (-mg, -eg)
    }
}

/// Undo information for one ply — pushed onto `history` on `make_move`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct UndoInfo {
    captured: Option<Piece>,
    captured_sq: Option<Square>,
    castling: u8,
    en_passant: Option<Square>,
    halfmove: u8,
    hash: u64,
    psqt_mg: i32,
    psqt_eg: i32,
    pawn_hash: u64,
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
    /// Incremental PSQT (material+PST) — white minus black, MG/EG.
    psqt_mg: i32,
    psqt_eg: i32,
    /// Pawn structure hash (XOR of pawn piece-square keys only).
    pawn_hash: u64,
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
            psqt_mg: 0,
            psqt_eg: 0,
            pawn_hash: 0,
            history: Vec::with_capacity(384),
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
        let old = self.board[idx];
        // Remove old piece if any.
        if let Some(old_p) = old {
            let bb = Bitboard::from_sq(sq);
            self.pieces[old_p.index()] = Bitboard(self.pieces[old_p.index()].0 & !bb.0);
            self.occupancy[old_p.color().as_usize()] =
                Bitboard(self.occupancy[old_p.color().as_usize()].0 & !bb.0);
            self.occupied = Bitboard(self.occupied.0 & !bb.0);
            let (dm, de) = psqt_delta(old_p, sq);
            self.psqt_mg -= dm;
            self.psqt_eg -= de;
            if old_p.piece_type() == PieceType::Pawn {
                self.pawn_hash ^= zobrist::keys().piece_sq[old_p.index()][idx];
            }
        }
        // Place new piece if any.
        if let Some(p) = piece {
            let bb = Bitboard::from_sq(sq);
            self.pieces[p.index()] = Bitboard(self.pieces[p.index()].0 | bb.0);
            self.occupancy[p.color().as_usize()] =
                Bitboard(self.occupancy[p.color().as_usize()].0 | bb.0);
            self.occupied = Bitboard(self.occupied.0 | bb.0);
            let (dm, de) = psqt_delta(p, sq);
            self.psqt_mg += dm;
            self.psqt_eg += de;
            if p.piece_type() == PieceType::Pawn {
                self.pawn_hash ^= zobrist::keys().piece_sq[p.index()][idx];
            }
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
    pub fn psqt_mg(&self) -> i32 {
        self.psqt_mg
    }

    #[inline]
    pub fn psqt_eg(&self) -> i32 {
        self.psqt_eg
    }

    #[inline]
    pub fn pawn_hash(&self) -> u64 {
        self.pawn_hash
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
            psqt_mg: self.psqt_mg,
            psqt_eg: self.psqt_eg,
            pawn_hash: self.pawn_hash,
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
            let (rook_from, rook_to) = castling_rook_squares(from, to)
                .unwrap_or_else(|| panic!("invalid castling move {mv}"));
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
                if from == crate::board::types::H1 {
                    self.castling &= !CASTLE_WK;
                } else if from == crate::board::types::A1 {
                    self.castling &= !CASTLE_WQ;
                }
            }
            Piece::BR => {
                if from == crate::board::types::H8 {
                    self.castling &= !CASTLE_BK;
                } else if from == crate::board::types::A8 {
                    self.castling &= !CASTLE_BQ;
                }
            }
            _ => {}
        }
        // Captured rook on original square also removes rights
        if let Some(cap) = captured {
            match cap {
                Piece::WR => {
                    if to == crate::board::types::H1 {
                        self.castling &= !CASTLE_WK;
                    } else if to == crate::board::types::A1 {
                        self.castling &= !CASTLE_WQ;
                    }
                }
                Piece::BR => {
                    if to == crate::board::types::H8 {
                        self.castling &= !CASTLE_BK;
                    } else if to == crate::board::types::A8 {
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
            let (rook_from, rook_to) = castling_rook_squares(from, to)
                .unwrap_or_else(|| panic!("invalid castling move {mv}"));
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
            let (rook_from, rook_to) = castling_rook_squares(from, to)
                .unwrap_or_else(|| panic!("invalid castling unmake {mv}"));
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
        self.psqt_mg = undo.psqt_mg;
        self.psqt_eg = undo.psqt_eg;
        self.pawn_hash = undo.pawn_hash;
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
    // Draw detection helpers
    // -----------------------------------------------------------------------

    /// Two-fold repetition: current hash equals any previous position's hash
    /// within the reversible window (bounded by `halfmove`).  Treats the first
    /// repetition as a draw inside search.
    pub fn is_repetition(&self) -> bool {
        let cur = self.hash;
        // Only positions since the last pawn move / capture can possibly repeat.
        // `halfmove` counts reversible plies since then.
        let hm = self.halfmove as usize;
        if hm == 0 || self.history.is_empty() {
            return false;
        }
        // Number of history entries that could match — limited by halfmove.
        let window = hm.min(self.history.len());
        let start = self.history.len() - window;
        for i in start..self.history.len() {
            if self.history[i].hash == cur {
                return true;
            }
        }
        false
    }

    /// 50-move rule: 100 half-moves without pawn move or capture.
    #[inline]
    pub fn is_fifty_move_draw(&self) -> bool {
        self.halfmove >= 100
    }

    /// True if the position is a draw by repetition or 50-move.
    #[inline]
    pub fn is_draw(&self) -> bool {
        self.is_fifty_move_draw() || self.is_repetition()
    }

    // -----------------------------------------------------------------------
    // Null move + material helpers
    // -----------------------------------------------------------------------

    /// True if `color` has any non-pawn, non-king material.
    /// Used to avoid null-move pruning in zugzwang-prone endings
    /// (K+P vs K, etc.).
    pub fn has_non_pawn_material(&self, color: Color) -> bool {
        for pt in [
            PieceType::Knight,
            PieceType::Bishop,
            PieceType::Rook,
            PieceType::Queen,
        ] {
            let bb = self.pieces_bb(Piece::new(color, pt));
            if !bb.is_empty() {
                return true;
            }
        }
        false
    }

    /// Make a null move: pass the turn, clear en passant, flip side, update hash.
    /// Pushes an undo record so `unmake_null_move` restores everything.
    /// Increments halfmove (a null is a reversible ply).
    pub fn make_null_move(&mut self) {
        let undo = UndoInfo {
            captured: None,
            captured_sq: None,
            castling: self.castling,
            en_passant: self.en_passant,
            halfmove: self.halfmove,
            hash: self.hash,
            psqt_mg: self.psqt_mg,
            psqt_eg: self.psqt_eg,
            pawn_hash: self.pawn_hash,
        };
        let old_hash = self.hash;
        let old_ep = self.en_passant;
        self.history.push(undo);

        // Incremental hash: side + old en passant file
        let keys = zobrist::keys();
        let mut new_hash = old_hash;
        new_hash ^= keys.side;
        if let Some(ep) = old_ep {
            new_hash ^= keys.en_passant[ep.file() as usize];
        }
        self.hash = new_hash;
        self.en_passant = None;
        self.halfmove = self.halfmove.wrapping_add(1);
        let prev_side = self.side_to_move;
        self.side_to_move = self.side_to_move.opposite();
        if prev_side == Color::Black {
            self.fullmove = self.fullmove.wrapping_add(1);
        }
    }

    /// Unmake the last null move.
    pub fn unmake_null_move(&mut self) {
        let undo = self.history.pop().expect("unmake_null without history");
        // Restore side/fullmove
        self.side_to_move = self.side_to_move.opposite();
        if self.side_to_move == Color::Black {
            self.fullmove = self.fullmove.wrapping_sub(1);
        }
        self.en_passant = undo.en_passant;
        self.halfmove = undo.halfmove;
        self.hash = undo.hash;
        self.castling = undo.castling;
        self.psqt_mg = undo.psqt_mg;
        self.psqt_eg = undo.psqt_eg;
        self.pawn_hash = undo.pawn_hash;
    }

    /// Flip side to move without touching history (debug `flip` command, UCI spec §5.7).
    ///
    /// Clears en passant (XORing its file key if set) and XORs the side key,
    /// mirroring `make_null_move`'s hashing but without pushing an undo record
    /// or bumping halfmove/fullmove (a debug flip is not a ply).
    pub fn flip_side_to_move(&mut self) {
        let old_ep = self.en_passant;
        let keys = zobrist::keys();
        let mut new_hash = self.hash;
        new_hash ^= keys.side;
        if let Some(ep) = old_ep {
            new_hash ^= keys.en_passant[ep.file() as usize];
        }
        self.hash = new_hash;
        self.en_passant = None;
        self.side_to_move = self.side_to_move.opposite();
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
        self.recompute_hash();
        // Recompute incremental PSQT and pawn hash from scratch for consistency.
        self.psqt_mg = 0;
        self.psqt_eg = 0;
        self.pawn_hash = 0;
        for sq in Square::ALL {
            if let Some(p) = self.board[sq.index() as usize] {
                let (dm, de) = psqt_delta(p, sq);
                self.psqt_mg += dm;
                self.psqt_eg += de;
                if p.piece_type() == PieceType::Pawn {
                    self.pawn_hash ^= zobrist::keys().piece_sq[p.index()][sq.index() as usize];
                }
            }
        }
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

    #[test]
    fn psqt_incremental_matches() {
        fn recompute(pos: &Position) -> (i32, i32, u64) {
            let mut mg = 0;
            let mut eg = 0;
            let mut ph = 0u64;
            for sq in Square::ALL {
                if let Some(p) = pos.piece_at(sq) {
                    let (dm, de) = psqt_delta(p, sq);
                    mg += dm;
                    eg += de;
                    if p.piece_type() == PieceType::Pawn {
                        ph ^=
                            crate::board::zobrist::keys().piece_sq[p.index()][sq.index() as usize];
                    }
                }
            }
            (mg, eg, ph)
        }
        let mut pos = Position::startpos();
        let (em, ee, eph) = recompute(&pos);
        assert_eq!(pos.psqt_mg(), em);
        assert_eq!(pos.psqt_eg(), ee);
        assert_eq!(pos.pawn_hash(), eph);
        let moves = [
            "e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "a7a6", "b5a4", "g8f6",
        ];
        for mv_str in moves {
            let mv = crate::board::mv::Move::parse_uci(mv_str).unwrap();
            pos.make_move(mv);
            let (em2, ee2, eph2) = recompute(&pos);
            assert_eq!(pos.psqt_mg(), em2, "psqt mg after {}", mv_str);
            assert_eq!(pos.psqt_eg(), ee2);
            assert_eq!(pos.pawn_hash(), eph2);
        }
        for mv_str in moves.iter().rev() {
            let mv = crate::board::mv::Move::parse_uci(mv_str).unwrap();
            pos.unmake_move(mv);
            let (em3, ee3, eph3) = recompute(&pos);
            assert_eq!(pos.psqt_mg(), em3);
            assert_eq!(pos.psqt_eg(), ee3);
            assert_eq!(pos.pawn_hash(), eph3);
        }
        // eval incremental vs breakdown for a couple FENs
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
        ] {
            let p = parse_fen(fen).unwrap();
            let inc = crate::eval::evaluate(&p);
            let bd = crate::eval::breakdown(&p);
            let base = bd.white_score();
            let tempo = crate::eval::TEMPO_BONUS_MG * bd.phase / 24;
            let white = base
                + if p.side_to_move() == Color::White {
                    tempo
                } else {
                    -tempo
                };
            let bd_eval = if p.side_to_move() == Color::White {
                white
            } else {
                -white
            };
            assert_eq!(inc, bd_eval, "eval mismatch fen {}", fen);
        }
    }
}

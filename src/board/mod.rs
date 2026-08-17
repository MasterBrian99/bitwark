#![allow(unused)]
//! Board representation — bitboards, position, FEN, zobrist.
//!
//! This module is the heart of the engine's state. Everything here is pure
//! synchronous Rust with no allocation in the hot path. See
//! chessprogramming.org “Bitboards” / “FEN” / “Zobrist Hashing”.

pub mod attacks;
pub mod fen;
pub mod position;
pub mod types;
pub mod zobrist;

pub use attacks::{king_attacks, knight_attacks, pawn_attacks};
pub use fen::{FenError, START_FEN, parse_fen, to_fen};
pub use position::{CASTLE_ALL, CASTLE_BK, CASTLE_BQ, CASTLE_WK, CASTLE_WQ, Position};
pub use types::{A1, A8, Bitboard, Color, E4, H1, H8, Piece, PieceType, Square};
pub use zobrist::keys as zobrist_keys;

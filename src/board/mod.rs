#![allow(unused)]
//! Board representation — bitboards, position, FEN, zobrist.
//!
//! This module is the heart of the engine's state. Everything here is pure
//! synchronous Rust with no allocation in the hot path. See
//! chessprogramming.org “Bitboards” / “FEN” / “Zobrist Hashing”.

pub mod attacks;
pub mod fen;
pub mod movgen;
pub mod mv;
pub mod perft;
pub mod position;
pub mod types;
pub mod zobrist;

pub use attacks::{
    bishop_attacks, king_attacks, knight_attacks, pawn_attacks, queen_attacks, rook_attacks,
};
pub use fen::{FenError, START_FEN, parse_fen, to_fen};
pub use movgen::{
    count_legal, generate_captures, generate_legal, generate_pseudo_captures,
    generate_pseudo_legal, is_square_attacked,
};
pub use mv::Move;
pub use perft::perft;
pub use position::{CASTLE_ALL, CASTLE_BK, CASTLE_BQ, CASTLE_WK, CASTLE_WQ, Position};
pub use types::{A1, A8, Bitboard, Color, E4, H1, H8, Piece, PieceType, Square};
pub use zobrist::keys as zobrist_keys;

#![allow(unused)]
//! Zobrist hashing — a fast, incremental board fingerprint.
//!
//! Every position gets a 64-bit `hash` that is the XOR of random numbers
//! for each feature (piece on square, side to move, castling rights, en
//! passant file). Moving a piece is just `hash ^= old ^ new` — O(1).
//! See chessprogramming.org “Zobrist Hashing”.
//!
//! The random table is generated once with a deterministic PRNG
//! (`Xorshift64`) so every run of the engine produces the same keys. This
//! makes `d` output and future `bench` signatures reproducible.

use std::sync::OnceLock;

use crate::board::types::{Piece, Square};

// ---------------------------------------------------------------------------
// PRNG
// ---------------------------------------------------------------------------

/// Minimal, deterministic PRNG — no deps, fast, good enough for Zobrist.
///
/// Xorshift64* (Vigna) — period 2^64-1, passes SmallCrush. Seed must be
/// non-zero. See chessprogramming.org “Pseudo Random Number Generator”.
#[derive(Debug, Clone)]
struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    const fn new(seed: u64) -> Self {
        // Zero seed would deadlock the xorshift.
        let s = if seed == 0 { 0x9E3779B97F4A7C15 } else { seed };
        Self { state: s }
    }

    #[inline]
    fn next(&mut self) -> u64 {
        // Xorshift64* — three shifts + multiply by a constant.
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// All random numbers used for hashing. Built once, shared immutably.
#[derive(Debug, Clone)]
pub struct ZobristKeys {
    /// `piece_sq[piece][square]` — 12 × 64.
    pub piece_sq: [[u64; 64]; 12],
    /// Side to move (only when Black to move; White's term is 0).
    pub side: u64,
    /// `castling[mask]` where mask is 4-bit KQkq (0..16).
    pub castling: [u64; 16],
    /// `en_passant[file]` (0..8, only file matters per Stockfish).
    pub en_passant: [u64; 8],
}

impl ZobristKeys {
    fn generate() -> Self {
        // Fixed seed — deterministic across runs and machines.
        let mut rng = Xorshift64::new(0x9E3779B97F4A7C15u64 ^ 0xCAFE1234u64);

        let mut piece_sq = [[0u64; 64]; 12];
        for row in &mut piece_sq {
            for slot in row.iter_mut() {
                let mut v = rng.next();
                // Avoid zero keys (would make XOR a no-op for that feature).
                if v == 0 {
                    v = 1;
                }
                *slot = v;
            }
        }

        let mut side = rng.next();
        if side == 0 {
            side = 1;
        }
        let mut castling = [0u64; 16];
        for (m, slot) in castling.iter_mut().enumerate() {
            let mut v = rng.next();
            if m != 0 && v == 0 {
                v = 1;
            }
            *slot = v;
        }
        castling[0] = 0; // no rights contributes nothing

        let mut en_passant = [0u64; 8];
        for slot in &mut en_passant {
            let mut v = rng.next();
            if v == 0 {
                v = 1;
            }
            *slot = v;
        }

        Self {
            piece_sq,
            side,
            castling,
            en_passant,
        }
    }

    /// Full recompute — used once per FEN parse and for testing.
    /// Incremental make/unmake updates XOR the same table entries.
    pub fn hash_position(
        &self,
        board: &[Option<Piece>; 64],
        side_to_move: crate::board::types::Color,
        castling: u8,
        en_passant: Option<Square>,
    ) -> u64 {
        let mut h = 0u64;
        for (sq_idx, maybe_piece) in board.iter().enumerate() {
            if let Some(p) = maybe_piece {
                h ^= self.piece_sq[p.index()][sq_idx];
            }
        }
        if side_to_move == crate::board::types::Color::Black {
            h ^= self.side;
        }
        h ^= self.castling[(castling & 0xF) as usize];
        if let Some(sq) = en_passant {
            h ^= self.en_passant[sq.file() as usize];
        }
        h
    }
}

static KEYS: OnceLock<ZobristKeys> = OnceLock::new();

/// Global Zobrist table — initialized on first use, then shared immutably.
///
/// `OnceLock` is `Sync` and lock-free after init, so this is free in the hot path.
pub fn keys() -> &'static ZobristKeys {
    KEYS.get_or_init(ZobristKeys::generate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determinism() {
        let k1 = ZobristKeys::generate();
        let k2 = ZobristKeys::generate();
        assert_eq!(k1.piece_sq[0][0], k2.piece_sq[0][0]);
        assert_eq!(k1.side, k2.side);
        assert_eq!(k1.castling[5], k2.castling[5]);
        assert_eq!(k1.en_passant[3], k2.en_passant[3]);
    }

    #[test]
    fn global_keys_singleton() {
        let a = keys() as *const ZobristKeys;
        let b = keys() as *const ZobristKeys;
        assert_eq!(a, b);
    }

    #[test]
    fn hash_nonzero() {
        let keys = keys();
        let board = [None; 64];
        let h = keys.hash_position(&board, crate::board::types::Color::White, 0, None);
        // Empty board white to move, no castling, no ep → 0
        assert_eq!(h, 0);
        let h2 = keys.hash_position(&board, crate::board::types::Color::Black, 0, None);
        assert_ne!(h2, 0); // side key
    }
}

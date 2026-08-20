//! MoveList — stack move buffer for the hot path.

use crate::board::Move;

/// Maximum legal moves in any chess position is 218 (per chessprogramming.org).
pub const MAX_MOVES: usize = 218;

#[derive(Clone)]
pub struct MoveList {
    pub moves: [Move; MAX_MOVES],
    pub scores: [i32; MAX_MOVES],
    pub len: usize,
}

impl Default for MoveList {
    fn default() -> Self {
        Self::new()
    }
}

impl MoveList {
    #[inline]
    pub fn new() -> Self {
        Self {
            moves: [Move {
                from: crate::board::Square(0),
                to: crate::board::Square(0),
                promotion: None,
            }; MAX_MOVES],
            scores: [0; MAX_MOVES],
            len: 0,
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    #[inline]
    pub fn push(&mut self, mv: Move) {
        debug_assert!(self.len < MAX_MOVES);
        self.moves[self.len] = mv;
        self.len += 1;
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Pick the best move from `from_idx..len` by score (max score wins,
    /// first-max on ties to preserve deterministic ordering). Swaps `moves` and
    /// `scores` in tandem and returns the move now at `from_idx`.
    #[inline]
    pub fn pick_best(&mut self, from_idx: usize) -> Move {
        debug_assert!(from_idx < self.len);
        let mut best_idx = from_idx;
        let mut best_score = self.scores[from_idx];
        for i in (from_idx + 1)..self.len {
            let s = self.scores[i];
            if s > best_score {
                best_score = s;
                best_idx = i;
            }
        }
        if best_idx != from_idx {
            self.moves.swap(from_idx, best_idx);
            self.scores.swap(from_idx, best_idx);
        }
        self.moves[from_idx]
    }
}

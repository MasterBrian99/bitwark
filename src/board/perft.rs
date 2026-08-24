#![allow(unused)]
//! Perft — move generation correctness and speed test.
//!
//! Perft counts leaf nodes at a given depth by fully expanding the move
//! tree. It is the standard move-generation oracle: if perft counts match
//! Stockfish on a suite of positions, movegen is almost certainly correct.
//! See chessprogramming.org “Perft”.

use crate::board::{Position, movgen};

/// Count leaf nodes at `depth` from `pos`.
///
/// `depth == 0` returns 1 (the current position). For `depth >= 1` we
/// generate all legal moves, make each, recurse, and unmake. At `depth == 1`
/// we bulk-count (just return move count) to avoid make/unmake overhead —
/// the classical 100M+ nps trick.
pub fn perft(pos: &mut Position, depth: u8) -> u64 {
    if depth == 0 {
        return 1;
    }
    if depth == 1 {
        return movgen::count_legal(pos) as u64;
    }

    let mut moves = Vec::new();
    movgen::generate_legal(pos, &mut moves);
    let mut nodes = 0u64;
    for mv in moves {
        pos.make_move(mv);
        nodes += perft(pos, depth - 1);
        pos.unmake_move(mv);
    }
    nodes
}

/// Perft divide — for `d` debugging: returns per-move counts for the root.
pub fn perft_divide(pos: &mut Position, depth: u8) -> Vec<(crate::board::mv::Move, u64)> {
    if depth == 0 {
        return Vec::new();
    }
    let mut moves = Vec::new();
    movgen::generate_legal(pos, &mut moves);
    let mut result = Vec::new();
    for mv in moves {
        pos.make_move(mv);
        let nodes = if depth == 1 { 1 } else { perft(pos, depth - 1) };
        pos.unmake_move(mv);
        result.push((mv, nodes));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::parse_fen;

    #[test]
    fn perft_startpos() {
        let mut pos =
            parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        assert_eq!(perft(&mut pos.clone(), 1), 20);
        assert_eq!(perft(&mut pos.clone(), 2), 400);
        assert_eq!(perft(&mut pos.clone(), 3), 8902);
        assert_eq!(perft(&mut pos.clone(), 4), 197281);
        assert_eq!(perft(&mut pos.clone(), 5), 4865609);
    }

    #[test]
    fn perft_kiwipete() {
        let mut pos =
            parse_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
                .unwrap();
        assert_eq!(perft(&mut pos.clone(), 1), 48);
        assert_eq!(perft(&mut pos.clone(), 2), 2039);
        assert_eq!(perft(&mut pos.clone(), 3), 97862);
    }

    #[test]
    fn perft_position3() {
        let mut pos = parse_fen("8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1").unwrap();
        assert_eq!(perft(&mut pos.clone(), 1), 14);
        assert_eq!(perft(&mut pos.clone(), 2), 191);
        assert_eq!(perft(&mut pos.clone(), 3), 2812);
    }
}

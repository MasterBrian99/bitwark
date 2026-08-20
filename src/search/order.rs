//! Move ordering — TT move → MVV-LVA captures → killers → history.
//!
//! A staged bucket scorer replaces the minimal MVV-LVA scheme;
//! Higher is searched first. See chessprogramming.org "Move Ordering",
//! "MVV-LVA", "Killer Heuristic", "History Heuristic".

use crate::board::movelist::MoveList;
use crate::board::types::Color;
use crate::board::{Move, PieceType, Position};

// Piece values for MVV-LVA (centipawns, same as eval material but King = 20000).
fn piece_value(pt: PieceType) -> i32 {
    match pt {
        PieceType::Pawn => 100,
        PieceType::Knight => 320,
        PieceType::Bishop => 330,
        PieceType::Rook => 500,
        PieceType::Queen => 900,
        PieceType::King => 20000,
    }
}

/// Score a move for ordering.  `pos` is the position *before* the move.
///
/// When `tt_move`, `killers`, and `history` are provided, quiet moves are
/// ordered by killers then history. Captures always outrank killers/history.
/// For qsearch, pass `tt_move=None` and dummy killers/history — capture
/// scoring still works.
pub fn score_move(
    pos: &Position,
    mv: Move,
    tt_move: Option<Move>,
    killers: &[Option<Move>; 2],
    history: &[[[i32; 64]; 64]; 2],
    _ply: usize,
) -> i32 {
    // TT move is proven — search it first.
    if let Some(tt) = tt_move
        && tt == mv
    {
        return 1_000_000;
    }

    let moving = match pos.piece_at(mv.from) {
        Some(p) => p,
        None => return 0,
    };
    let attacker_val = piece_value(moving.piece_type());

    // Captured piece, if any (including en passant).
    let captured = if let Some(p) = pos.piece_at(mv.to) {
        Some(p)
    } else if moving.piece_type() == PieceType::Pawn && pos.en_passant() == Some(mv.to) {
        // En passant captures a pawn — victim value 100.
        let promo_bonus = mv.promotion.map_or(0, piece_value);
        let mvv = 10 * 100 - attacker_val + promo_bonus;
        return 800_000 + mvv;
    } else {
        None
    };

    if let Some(cap) = captured {
        let victim_val = piece_value(cap.piece_type());
        let promo_bonus = mv.promotion.map_or(0, piece_value);
        let mvv = 10 * victim_val - attacker_val + promo_bonus;
        return 800_000 + mvv;
    }

    // Non-capture promotion (quiet promo).
    if let Some(pt) = mv.promotion {
        return 600_000 + piece_value(pt);
    }

    // Quiet move — check killers then history.
    if killers[0] == Some(mv) {
        return 500_000;
    }
    if killers[1] == Some(mv) {
        return 490_000;
    }
    // History: side × from × to
    let side = pos.side_to_move() as usize;
    let h = history[side][mv.from.index() as usize][mv.to.index() as usize];
    // Clamp history to a bucket below killers: killers are 490k+, history max 16k
    // so history quiets never outrank a killer, but high-history quiets beat
    // low-history quiets.
    h.clamp(-16_384, 16_384)
}

/// Backwards-compatible scorer for qsearch/captures-only callers.
/// TT move is `None`, killers empty, history zeroed — captures scored by MVV-LVA,
/// quiets 0.
#[allow(dead_code)]
pub fn score_move_simple(pos: &Position, mv: Move) -> i32 {
    let dummy_killers = [None, None];
    let dummy_history = [[[0; 64]; 64]; 2];
    score_move(pos, mv, None, &dummy_killers, &dummy_history, 0)
}

/// Bulk-score a MoveList into its parallel `scores` array (one pass, O(n)).
#[inline]
pub fn score_list(
    pos: &Position,
    list: &mut MoveList,
    tt_move: Option<Move>,
    killers: &[Option<Move>; 2],
    history: &[[[i32; 64]; 64]; 2],
    ply: usize,
) {
    for i in 0..list.len {
        list.scores[i] = score_move(pos, list.moves[i], tt_move, killers, history, ply);
    }
}

/// Update killers on a quiet beta cutoff.
#[inline]
pub fn update_killers(killers: &mut [[Option<Move>; 2]; 128], ply: usize, mv: Move) {
    if ply >= killers.len() {
        return;
    }
    let slot = &mut killers[ply];
    if slot[0] != Some(mv) {
        slot[1] = slot[0];
        slot[0] = Some(mv);
    }
}

/// Update history on a quiet beta cutoff. Gravity with cap ±16384.
#[inline]
pub fn update_history(history: &mut [[[i32; 64]; 64]; 2], side: Color, mv: Move, depth: i32) {
    let s = side as usize;
    let from = mv.from.index() as usize;
    let to = mv.to.index() as usize;
    let bonus = (depth * depth).clamp(0, 16384);
    let entry = &mut history[s][from][to];
    // gravity: entry += bonus - entry*bonus/16384
    *entry += bonus - *entry * bonus / 16384;
    *entry = (*entry).clamp(-16384, 16384);
    // Halve all entries on overflow is handled by clamping; a full halve
    // across the table is rare enough to do lazily when any entry hits cap.
    // Check if this entry hit the cap and halve the whole table.
    if entry.abs() >= 16384 {
        for side_hist in history.iter_mut() {
            for from_hist in side_hist.iter_mut() {
                for v in from_hist.iter_mut() {
                    *v /= 2;
                }
            }
        }
    }
}

/// Find the index of the highest-scored move in `moves` according to
/// `score_move`, from `pos` before the move.
#[allow(dead_code)]
pub fn pick_best(
    moves: &[Move],
    pos: &Position,
    tt_move: Option<Move>,
    killers: &[Option<Move>; 2],
    history: &[[[i32; 64]; 64]; 2],
    ply: usize,
) -> usize {
    debug_assert!(!moves.is_empty());
    let mut best_idx = 0;
    let mut best_score = score_move(pos, moves[0], tt_move, killers, history, ply);
    for (i, &mv) in moves.iter().enumerate().skip(1) {
        let s = score_move(pos, mv, tt_move, killers, history, ply);
        if s > best_score {
            best_score = s;
            best_idx = i;
        }
    }
    best_idx
}

/// Sort `moves` in-place descending by the full ordering.  Used for qsearch
/// and debugging.  The main search uses incremental `pick_best` to avoid
/// sorting the tail that will be pruned.
#[allow(dead_code)]
pub fn sort_moves(
    moves: &mut [Move],
    pos: &Position,
    tt_move: Option<Move>,
    killers: &[Option<Move>; 2],
    history: &[[[i32; 64]; 64]; 2],
    ply: usize,
) {
    moves.sort_by_key(|&mv| std::cmp::Reverse(score_move(pos, mv, tt_move, killers, history, ply)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::parse_fen;

    #[test]
    fn mvv_lva_capture_order() {
        let pos2 = parse_fen("3rk3/8/8/4p3/8/8/8/3QK3 w - - 0 1").unwrap();
        let mv_qxr = crate::board::mv::Move::parse_uci("d1d8").unwrap(); // Q x R
        let mv_qxp = crate::board::mv::Move::parse_uci("d1e2").unwrap(); // quiet
        let dummy_k = [None, None];
        let dummy_h = [[[0; 64]; 64]; 2];
        assert!(
            score_move(&pos2, mv_qxr, None, &dummy_k, &dummy_h, 0)
                > score_move(&pos2, mv_qxp, None, &dummy_k, &dummy_h, 0)
        );
        let mv_qxp2 = {
            let p = parse_fen("4k3/8/8/4p3/3Q4/8/8/4K3 w - - 0 1").unwrap();
            let mv = crate::board::mv::Move::parse_uci("d4e5").unwrap();
            score_move(&p, mv, None, &dummy_k, &dummy_h, 0)
        };
        let score_rook_cap = score_move(&pos2, mv_qxr, None, &dummy_k, &dummy_h, 0);
        assert!(score_rook_cap > mv_qxp2);
    }

    #[test]
    fn promo_scores_high() {
        let pos = parse_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let promo_q = crate::board::mv::Move::parse_uci("a7a8q").unwrap();
        let dummy_k = [None, None];
        let dummy_h = [[[0; 64]; 64]; 2];
        assert!(score_move(&pos, promo_q, None, &dummy_k, &dummy_h, 0) > 0);
    }

    #[test]
    fn tt_move_first() {
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let tt = crate::board::mv::Move::parse_uci("g1f3").unwrap();
        let quiet = crate::board::mv::Move::parse_uci("b1c3").unwrap();
        let cap = crate::board::mv::Move::parse_uci("g1f3").unwrap(); // same as tt = capture? not capture but quiet; choose ep?
        let dummy_k = [None, None];
        let dummy_h = [[[0; 64]; 64]; 2];
        let s_tt = score_move(&pos, tt, Some(tt), &dummy_k, &dummy_h, 0);
        let s_q = score_move(&pos, quiet, Some(tt), &dummy_k, &dummy_h, 0);
        assert!(s_tt > s_q);
        // Even a winning capture should lose to TT move
        let pos2 = parse_fen("3rk3/8/8/4p3/8/8/8/3QK3 w - - 0 1").unwrap();
        let qxr = crate::board::mv::Move::parse_uci("d1d8").unwrap();
        let s_cap = score_move(&pos2, qxr, Some(quiet), &dummy_k, &dummy_h, 0);
        let s_tt2 = score_move(&pos2, quiet, Some(quiet), &dummy_k, &dummy_h, 0);
        assert!(s_tt2 > s_cap);
    }

    #[test]
    fn killer_above_history() {
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let k_move = crate::board::mv::Move::parse_uci("g1f3").unwrap();
        let h_move = crate::board::mv::Move::parse_uci("b1c3").unwrap();
        let killers = [Some(k_move), None];
        let mut history = [[[0; 64]; 64]; 2];
        // Give h_move max history
        history[Color::White as usize][h_move.from.index() as usize][h_move.to.index() as usize] =
            16384;
        let s_k = score_move(&pos, k_move, None, &killers, &history, 0);
        let s_h = score_move(&pos, h_move, None, &killers, &history, 0);
        assert!(s_k > s_h, "killer should outrank even max history");
    }

    #[test]
    fn history_update_gravity() {
        let mut history = [[[0; 64]; 64]; 2];
        let mv = crate::board::mv::Move::parse_uci("e2e4").unwrap();
        update_history(&mut history, Color::White, mv, 6);
        let v = history[Color::White as usize][mv.from.index() as usize][mv.to.index() as usize];
        assert!(v > 0);
        // Repeated updates should approach cap, not overflow
        for _ in 0..200 {
            update_history(&mut history, Color::White, mv, 10);
        }
        let v2 = history[Color::White as usize][mv.from.index() as usize][mv.to.index() as usize];
        assert!(v2.abs() <= 16384);
    }
}

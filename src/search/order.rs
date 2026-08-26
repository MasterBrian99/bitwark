//! Move ordering — TT move → MVV-LVA captures → killers → countermove → history.
//!
//! A staged bucket scorer replaces the minimal MVV-LVA scheme;
//! countermoves (`[piece][to]`) and continuation
//! history (1-ply + 2-ply) add depth. Higher is searched first. See chessprogramming.org
//! "Move Ordering", "MVV-LVA", "Killer Heuristic", "History Heuristic",
//! "Countermoves", "Continuation History".

#![allow(clippy::too_many_arguments)]

use crate::board::movelist::MoveList;
use crate::board::types::{Color, Square};
use crate::board::{Move, Piece, PieceType, Position};
use crate::search::ContHistory;

pub static ZERO_CONT: ContHistory = [[[[0; 64]; 6]; 64]; 6];

/// Base score for losing captures (SEE < 0) after demotion; history quiets are ±16384.
#[allow(dead_code)]
pub const LOSING_CAPTURE_BASE: i32 = -20_000;
/// Scores below this were already SEE-checked and demoted (see value in score).
#[allow(dead_code)]
pub const DEMOTE_MARK: i32 = -19_000;

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

#[inline]
fn is_quiet_for_order(pos: &Position, mv: Move) -> bool {
    if mv.promotion.is_some() {
        return false;
    }
    if pos.piece_at(mv.to).is_some() {
        return false;
    }
    if let Some(pc) = pos.piece_at(mv.from) {
        if pc.piece_type() == PieceType::Pawn && pos.en_passant() == Some(mv.to) {
            return false;
        }
    }
    true
}

/// Score a move for ordering.  `pos` is the position *before* the move.
///
/// When `tt_move`, `killers`, `counter` and `history` are provided, quiet moves are
/// ordered by killers, then countermove (`~480k`), then history. Captures always
/// outrank killers/history. For qsearch, pass `tt_move=None` and dummy
/// killers/history/counter — capture scoring still works.
pub fn score_move(
    pos: &Position,
    mv: Move,
    tt_move: Option<Move>,
    killers: &[Option<Move>; 2],
    counter: Option<Move>,
    history: &[[[i32; 64]; 64]; 2],
    prev1: Option<(PieceType, Square)>,
    cont1: &ContHistory,
    prev2: Option<(PieceType, Square)>,
    cont2: &ContHistory,
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
        // En passant captures a pawn.
        Some(Piece::new(moving.color().opposite(), PieceType::Pawn))
    } else {
        None
    };

    if let Some(cap) = captured {
        let victim_val = piece_value(cap.piece_type());
        let promo_bonus = mv.promotion.map_or(0, piece_value);
        let mvv = 10 * victim_val - attacker_val + promo_bonus;
        // Phase 2: SEE-free — all captures score 800k + MVV, losing ones are
        // demoted lazily at pick time via LOSING_CAPTURE_BASE + see.
        return 800_000 + mvv;
    }

    // Non-capture promotion (quiet promo).
    if let Some(pt) = mv.promotion {
        return 600_000 + piece_value(pt);
    }

    // Quiet move — check killers, then countermove (~480k), then history.
    if killers[0] == Some(mv) {
        return 500_000;
    }
    if killers[1] == Some(mv) {
        return 490_000;
    }
    if let Some(cm) = counter
        && cm == mv
        && is_quiet_for_order(pos, mv)
    {
        return 480_000;
    }
    // History + continuation: max of butterfly and 1-ply/2-ply cont
    let side = pos.side_to_move() as usize;
    let h = history[side][mv.from.index() as usize][mv.to.index() as usize];
    let cur_pt = moving.piece_type();
    let mut best = h;
    if let Some((pp, ps)) = prev1 {
        let c1 = cont1[pp as usize][ps.index() as usize][cur_pt as usize][mv.to.index() as usize];
        if c1 > best {
            best = c1;
        }
    }
    if let Some((pp, ps)) = prev2 {
        let c2 = cont2[pp as usize][ps.index() as usize][cur_pt as usize][mv.to.index() as usize];
        if c2 > best {
            best = c2;
        }
    }
    // Clamp to a bucket below killers/counter: 490k/480k > 16k
    best.clamp(-16_384, 16_384)
}

/// SEE-aware variant (old score_move) — used only for root ordering to
/// keep bit-identical decisions. Interior nodes and qsearch use the SEE-free
/// `score_move` with lazy demotion at pick time (Phase 2).
pub fn score_move_with_see(
    pos: &Position,
    mv: Move,
    tt_move: Option<Move>,
    killers: &[Option<Move>; 2],
    counter: Option<Move>,
    history: &[[[i32; 64]; 64]; 2],
    prev1: Option<(PieceType, Square)>,
    cont1: &ContHistory,
    prev2: Option<(PieceType, Square)>,
    cont2: &ContHistory,
    ply: usize,
) -> i32 {
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
    let captured = if let Some(p) = pos.piece_at(mv.to) {
        Some(p)
    } else if moving.piece_type() == PieceType::Pawn && pos.en_passant() == Some(mv.to) {
        Some(Piece::new(moving.color().opposite(), PieceType::Pawn))
    } else {
        None
    };
    if let Some(cap) = captured {
        let victim_val = piece_value(cap.piece_type());
        let promo_bonus = mv.promotion.map_or(0, piece_value);
        let mvv = 10 * victim_val - attacker_val + promo_bonus;
        let see_val = crate::search::see::see(pos, mv);
        if see_val >= 0 {
            return 800_000 + mvv;
        } else {
            return LOSING_CAPTURE_BASE + see_val;
        }
    }
    if let Some(pt) = mv.promotion {
        return 600_000 + piece_value(pt);
    }
    if killers[0] == Some(mv) {
        return 500_000;
    }
    if killers[1] == Some(mv) {
        return 490_000;
    }
    if let Some(cm) = counter
        && cm == mv
        && is_quiet_for_order(pos, mv)
    {
        return 480_000;
    }
    let side = pos.side_to_move() as usize;
    let h = history[side][mv.from.index() as usize][mv.to.index() as usize];
    let cur_pt = moving.piece_type();
    let mut best = h;
    if let Some((pp, ps)) = prev1 {
        let c1 = cont1[pp as usize][ps.index() as usize][cur_pt as usize][mv.to.index() as usize];
        if c1 > best {
            best = c1;
        }
    }
    if let Some((pp, ps)) = prev2 {
        let c2 = cont2[pp as usize][ps.index() as usize][cur_pt as usize][mv.to.index() as usize];
        if c2 > best {
            best = c2;
        }
    }
    best.clamp(-16_384, 16_384)
}

/// Backwards-compatible scorer for qsearch/captures-only callers.
/// TT move is `None`, killers empty, history zeroed — captures scored by MVV-LVA,
/// quiets 0.
#[allow(dead_code)]
pub fn score_move_simple(pos: &Position, mv: Move) -> i32 {
    let dummy_killers = [None, None];
    let dummy_history = [[[0; 64]; 64]; 2];
    score_move(
        pos,
        mv,
        None,
        &dummy_killers,
        None,
        &dummy_history,
        None,
        &ZERO_CONT,
        None,
        &ZERO_CONT,
        0,
    )
}

/// Bulk-score a MoveList into its parallel `scores` array (one pass, O(n)).
#[inline]
pub fn score_list(
    pos: &Position,
    list: &mut MoveList,
    tt_move: Option<Move>,
    killers: &[Option<Move>; 2],
    counter: Option<Move>,
    history: &[[[i32; 64]; 64]; 2],
    prev1: Option<(PieceType, Square)>,
    cont1: &ContHistory,
    prev2: Option<(PieceType, Square)>,
    cont2: &ContHistory,
    ply: usize,
) {
    for i in 0..list.len {
        list.scores[i] = score_move(
            pos,
            list.moves[i],
            tt_move,
            killers,
            counter,
            history,
            prev1,
            cont1,
            prev2,
            cont2,
            ply,
        );
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

#[inline]
fn gravity(entry: &mut i32, bonus: i32) {
    *entry += bonus - *entry * bonus / 16384;
    *entry = (*entry).clamp(-16384, 16384);
}

/// Update history on a quiet beta cutoff. Per-entry gravity (10c): no global halve.
#[inline]
pub fn update_history(history: &mut [[[i32; 64]; 64]; 2], side: Color, mv: Move, depth: i32) {
    let s = side as usize;
    let from = mv.from.index() as usize;
    let to = mv.to.index() as usize;
    let bonus = (depth * depth).clamp(0, 16384);
    let entry = &mut history[s][from][to];
    gravity(entry, bonus);
}

/// Apply signed bonus/malus to a single history entry (10c: malus = -bonus/2).
#[inline]
pub fn update_history_with_bonus(
    history: &mut [[[i32; 64]; 64]; 2],
    side: Color,
    mv: Move,
    bonus: i32,
) {
    let s = side as usize;
    let entry = &mut history[s][mv.from.index() as usize][mv.to.index() as usize];
    gravity(entry, bonus);
}

/// Update continuation history entry with signed bonus (10b/c).
#[inline]
pub fn update_cont(
    cont: &mut ContHistory,
    prev_pt: PieceType,
    prev_sq: Square,
    cur_pt: PieceType,
    cur_to: Square,
    bonus: i32,
) {
    let e = &mut cont[prev_pt as usize][prev_sq.index() as usize][cur_pt as usize]
        [cur_to.index() as usize];
    gravity(e, bonus);
}

/// Find the index of the highest-scored move in `moves` according to
/// `score_move`, from `pos` before the move.
#[allow(dead_code)]
pub fn pick_best(
    moves: &[Move],
    pos: &Position,
    tt_move: Option<Move>,
    killers: &[Option<Move>; 2],
    counter: Option<Move>,
    history: &[[[i32; 64]; 64]; 2],
    prev1: Option<(PieceType, Square)>,
    cont1: &ContHistory,
    prev2: Option<(PieceType, Square)>,
    cont2: &ContHistory,
    ply: usize,
) -> usize {
    debug_assert!(!moves.is_empty());
    let mut best_idx = 0;
    let mut best_score = score_move(
        pos, moves[0], tt_move, killers, counter, history, prev1, cont1, prev2, cont2, ply,
    );
    for (i, &mv) in moves.iter().enumerate().skip(1) {
        let s = score_move(
            pos, mv, tt_move, killers, counter, history, prev1, cont1, prev2, cont2, ply,
        );
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
    counter: Option<Move>,
    history: &[[[i32; 64]; 64]; 2],
    prev1: Option<(PieceType, Square)>,
    cont1: &ContHistory,
    prev2: Option<(PieceType, Square)>,
    cont2: &ContHistory,
    ply: usize,
) {
    moves.sort_by_key(|&mv| {
        std::cmp::Reverse(score_move(
            pos, mv, tt_move, killers, counter, history, prev1, cont1, prev2, cont2, ply,
        ))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::parse_fen;

    #[test]
    fn mvv_lva_capture_order() {
        // Use undefended captures so SEE is winning and they outrank quiets.
        let pos2 = parse_fen("r3k3/8/8/8/8/8/8/Q3K3 w - - 0 1").unwrap();
        let mv_qxr = crate::board::mv::Move::parse_uci("a1a8").unwrap(); // Q x R
        let mv_qxp = crate::board::mv::Move::parse_uci("a1b2").unwrap(); // quiet
        let dummy_k = [None, None];
        let dummy_h = [[[0; 64]; 64]; 2];
        assert!(
            score_move(
                &pos2, mv_qxr, None, &dummy_k, None, &dummy_h, None, &ZERO_CONT, None, &ZERO_CONT,
                0
            ) > score_move(
                &pos2, mv_qxp, None, &dummy_k, None, &dummy_h, None, &ZERO_CONT, None, &ZERO_CONT,
                0
            )
        );
        let mv_qxp2 = {
            let p = parse_fen("4k3/8/8/4p3/3Q4/8/8/4K3 w - - 0 1").unwrap();
            let mv = crate::board::mv::Move::parse_uci("d4e5").unwrap();
            score_move(
                &p, mv, None, &dummy_k, None, &dummy_h, None, &ZERO_CONT, None, &ZERO_CONT, 0,
            )
        };
        let score_rook_cap = score_move(
            &pos2, mv_qxr, None, &dummy_k, None, &dummy_h, None, &ZERO_CONT, None, &ZERO_CONT, 0,
        );
        assert!(score_rook_cap > mv_qxp2);
    }

    #[test]
    fn see_losing_capture_below_history() {
        // Queen takes pawn defended by pawn — losing per SEE. With the old
        // SEE-bucketed scoring it sorted below quiets; with Phase 2's SEE-free
        // scoring it scores 800k+MVV and is demoted lazily at pick time.
        let pos = parse_fen("4k3/8/2p1p3/3p4/3Q4/8/8/4K3 w - - 0 1").unwrap();
        let losing = crate::board::mv::Move::parse_uci("d4d5").unwrap(); // QxP defended
        let quiet = crate::board::mv::Move::parse_uci("d4e4").unwrap();
        let dummy_k = [None, None];
        let mut history = [[[0; 64]; 64]; 2];
        // Give quiet max history
        history[Color::White as usize][quiet.from.index() as usize][quiet.to.index() as usize] =
            16384;
        // SEE-aware scoring (root/with_see) puts losing below quiet
        let s_losing_see = score_move_with_see(
            &pos, losing, None, &dummy_k, None, &history, None, &ZERO_CONT, None, &ZERO_CONT, 0,
        );
        let s_quiet = score_move(
            &pos, quiet, None, &dummy_k, None, &history, None, &ZERO_CONT, None, &ZERO_CONT, 0,
        );
        assert!(
            s_losing_see < s_quiet,
            "SEE-aware losing capture {s_losing_see} should be below history quiet {s_quiet}"
        );
        // SEE-free scoring (negamax/qsearch hot path) puts it in the winning bucket
        let s_losing_free = score_move(
            &pos, losing, None, &dummy_k, None, &history, None, &ZERO_CONT, None, &ZERO_CONT, 0,
        );
        assert!(
            s_losing_free > 700_000,
            "SEE-free losing capture should score 800k+MVV, got {s_losing_free}"
        );
        // But winning capture should still be above killer
        let pos_win = parse_fen("r3k3/8/8/8/8/8/8/Q3K3 w - - 0 1").unwrap();
        let winning = crate::board::mv::Move::parse_uci("a1a8").unwrap();
        let killers = [Some(quiet), None];
        let s_win = score_move(
            &pos_win, winning, None, &killers, None, &history, None, &ZERO_CONT, None, &ZERO_CONT,
            0,
        );
        let s_killer = score_move(
            &pos_win, quiet, None, &killers, None, &history, None, &ZERO_CONT, None, &ZERO_CONT, 0,
        );
        assert!(s_win > s_killer, "winning capture should outrank killer");
    }

    #[test]
    fn promo_scores_high() {
        let pos = parse_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let promo_q = crate::board::mv::Move::parse_uci("a7a8q").unwrap();
        let dummy_k = [None, None];
        let dummy_h = [[[0; 64]; 64]; 2];
        assert!(
            score_move(
                &pos, promo_q, None, &dummy_k, None, &dummy_h, None, &ZERO_CONT, None, &ZERO_CONT,
                0
            ) > 0
        );
    }

    #[test]
    fn tt_move_first() {
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let tt = crate::board::mv::Move::parse_uci("g1f3").unwrap();
        let quiet = crate::board::mv::Move::parse_uci("b1c3").unwrap();
        let cap = crate::board::mv::Move::parse_uci("g1f3").unwrap(); // same as tt = capture? not capture but quiet; choose ep?
        let dummy_k = [None, None];
        let dummy_h = [[[0; 64]; 64]; 2];
        let s_tt = score_move(
            &pos,
            tt,
            Some(tt),
            &dummy_k,
            None,
            &dummy_h,
            None,
            &ZERO_CONT,
            None,
            &ZERO_CONT,
            0,
        );
        let s_q = score_move(
            &pos,
            quiet,
            Some(tt),
            &dummy_k,
            None,
            &dummy_h,
            None,
            &ZERO_CONT,
            None,
            &ZERO_CONT,
            0,
        );
        assert!(s_tt > s_q);
        // Even a winning capture should lose to TT move
        let pos2 = parse_fen("3rk3/8/8/4p3/8/8/8/3QK3 w - - 0 1").unwrap();
        let qxr = crate::board::mv::Move::parse_uci("d1d8").unwrap();
        let s_cap = score_move(
            &pos2,
            qxr,
            Some(quiet),
            &dummy_k,
            None,
            &dummy_h,
            None,
            &ZERO_CONT,
            None,
            &ZERO_CONT,
            0,
        );
        let s_tt2 = score_move(
            &pos2,
            quiet,
            Some(quiet),
            &dummy_k,
            None,
            &dummy_h,
            None,
            &ZERO_CONT,
            None,
            &ZERO_CONT,
            0,
        );
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
        let s_k = score_move(
            &pos, k_move, None, &killers, None, &history, None, &ZERO_CONT, None, &ZERO_CONT, 0,
        );
        let s_h = score_move(
            &pos, h_move, None, &killers, None, &history, None, &ZERO_CONT, None, &ZERO_CONT, 0,
        );
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

    #[test]
    fn countermove_ranked_between_killer_and_history() {
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let k_move = crate::board::mv::Move::parse_uci("g1f3").unwrap();
        let c_move = crate::board::mv::Move::parse_uci("b1c3").unwrap();
        let h_move = crate::board::mv::Move::parse_uci("g1h3").unwrap();
        let killers = [Some(k_move), None];
        let mut history = [[[0; 64]; 64]; 2];
        history[Color::White as usize][h_move.from.index() as usize][h_move.to.index() as usize] =
            16384;
        // killer > counter > max history
        let s_k = score_move(
            &pos,
            k_move,
            None,
            &killers,
            Some(c_move),
            &history,
            None,
            &ZERO_CONT,
            None,
            &ZERO_CONT,
            0,
        );
        let s_c = score_move(
            &pos,
            c_move,
            None,
            &killers,
            Some(c_move),
            &history,
            None,
            &ZERO_CONT,
            None,
            &ZERO_CONT,
            0,
        );
        let s_h = score_move(
            &pos,
            h_move,
            None,
            &killers,
            Some(c_move),
            &history,
            None,
            &ZERO_CONT,
            None,
            &ZERO_CONT,
            0,
        );
        assert!(s_k > s_c, "killer should outrank countermove");
        assert!(s_c > s_h, "countermove should outrank max-history quiet");
        assert_eq!(s_c, 480_000);
    }

    #[test]
    fn countermove_only_for_quiets() {
        // Countermove that is a capture in this position should score as capture, not 480k
        let pos = parse_fen("r3k3/8/8/3p4/4Q3/8/8/4K3 w - - 0 1").unwrap();
        let cap = crate::board::mv::Move::parse_uci("e4d5").unwrap(); // QxP
        assert!(pos.piece_at(cap.to).is_some(), "should be capture");
        let killers = [None, None];
        let history = [[[0; 64]; 64]; 2];
        let s = score_move(
            &pos,
            cap,
            None,
            &killers,
            Some(cap),
            &history,
            None,
            &ZERO_CONT,
            None,
            &ZERO_CONT,
            0,
        );
        // Winning capture is 800k+..., not 480k
        assert!(
            s >= 800_000,
            "capture should be winning-capture bucket, got {s}"
        );
    }

    #[test]
    fn continuation_history_outranks_plain_history() {
        // Position with quiets; seed cont1 high, history low, verify cont wins
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        // Choose two quiet knight moves
        let mv_high = crate::board::mv::Move::parse_uci("g1f3").unwrap();
        let mv_low = crate::board::mv::Move::parse_uci("b1c3").unwrap();
        let killers = [None, None];
        let history = [[[0; 64]; 64]; 2];
        // prev1 = Pawn to e4, cur = Knight to f3 vs c3
        let prev_pt = PieceType::Pawn;
        let prev_sq = crate::board::types::Square::from_str("e4").unwrap();
        let mut cont1: ContHistory = [[[[0; 64]; 6]; 64]; 6];
        let cur_pt = PieceType::Knight;
        cont1[prev_pt as usize][prev_sq.index() as usize][cur_pt as usize]
            [mv_high.to.index() as usize] = 8000;
        cont1[prev_pt as usize][prev_sq.index() as usize][cur_pt as usize]
            [mv_low.to.index() as usize] = 100;
        let s_high = score_move(
            &pos,
            mv_high,
            None,
            &killers,
            None,
            &history,
            Some((prev_pt, prev_sq)),
            &cont1,
            None,
            &ZERO_CONT,
            0,
        );
        let s_low = score_move(
            &pos,
            mv_low,
            None,
            &killers,
            None,
            &history,
            Some((prev_pt, prev_sq)),
            &cont1,
            None,
            &ZERO_CONT,
            0,
        );
        assert!(s_high > s_low, "cont history should rank higher");
        assert_eq!(s_high, 8000);
        // With no prev, should be 0 (history zero)
        let s_no_prev = score_move(
            &pos, mv_high, None, &killers, None, &history, None, &cont1, None, &ZERO_CONT, 0,
        );
        assert_eq!(s_no_prev, 0);
    }

    #[test]
    fn cont_gravity_bounds() {
        let mut cont: ContHistory = [[[[0; 64]; 6]; 64]; 6];
        let pp = PieceType::Pawn;
        let ps = crate::board::types::Square::from_str("e4").unwrap();
        let cp = PieceType::Knight;
        let cs = crate::board::types::Square::from_str("f3").unwrap();
        // Simulate gravity updates like update_cont would do
        for _ in 0..200 {
            let bonus = 100; // depth 10
            let e = &mut cont[pp as usize][ps.index() as usize][cp as usize][cs.index() as usize];
            *e += bonus - *e * bonus / 16384;
            *e = (*e).clamp(-16384, 16384);
        }
        let v = cont[pp as usize][ps.index() as usize][cp as usize][cs.index() as usize];
        assert!(v.abs() <= 16384);
        assert!(v > 0);
    }

    #[test]
    fn history_malus_lowers() {
        let mut history = [[[0; 64]; 64]; 2];
        let mv_good = crate::board::mv::Move::parse_uci("e2e4").unwrap();
        let mv_bad = crate::board::mv::Move::parse_uci("g1f3").unwrap();
        // Bonus the good move
        update_history(&mut history, Color::White, mv_good, 10);
        let v_good = history[Color::White as usize][mv_good.from.index() as usize]
            [mv_good.to.index() as usize];
        assert!(v_good > 0);
        // Malus the bad move
        update_history_with_bonus(&mut history, Color::White, mv_bad, -50);
        let v_bad = history[Color::White as usize][mv_bad.from.index() as usize]
            [mv_bad.to.index() as usize];
        assert!(v_bad < 0, "malus should make entry negative, got {v_bad}");
    }

    #[test]
    fn no_global_halve_on_cap() {
        let mut history = [[[0; 64]; 64]; 2];
        // Fill other entry high but not capped
        let mv_a = crate::board::mv::Move::parse_uci("e2e4").unwrap();
        let mv_b = crate::board::mv::Move::parse_uci("d2d4").unwrap();
        // Seed mv_a to 8000 via repeated updates
        for _ in 0..50 {
            update_history(&mut history, Color::White, mv_a, 10);
        }
        let before =
            history[Color::White as usize][mv_a.from.index() as usize][mv_a.to.index() as usize];
        // Now push mv_b to cap (many updates)
        for _ in 0..300 {
            update_history(&mut history, Color::White, mv_b, 10);
        }
        let after_a =
            history[Color::White as usize][mv_a.from.index() as usize][mv_a.to.index() as usize];
        // With per-entry gravity (10c), hitting cap on mv_b should NOT halve mv_a
        assert_eq!(
            before, after_a,
            "global halve should be gone; other entries must stay"
        );
        let v_b =
            history[Color::White as usize][mv_b.from.index() as usize][mv_b.to.index() as usize];
        assert!(v_b.abs() <= 16384);
    }
}

//! Negamax alpha-beta with PV table, TT, pruning, and extensions.
//!
//! Null-move pruning, late-move reductions with PVS re-search,
//! futility / late-move pruning, and check extensions, plus aspiration at
//! the iterative-deepening level. See chessprogramming.org
//! "Alpha-Beta", "Transposition Table", "Null Move Pruning",
//! "Late Move Reductions", "Futility Pruning", "Check Extensions".

#![allow(clippy::needless_range_loop)]
#![allow(clippy::collapsible_if)]

use std::sync::OnceLock;
use std::sync::atomic::Ordering;

use crate::board::movelist::MoveList;
use crate::board::types::Bitboard;
use crate::board::{
    Move, Piece, PieceType, Position, Square, generate_legal_into, is_square_attacked,
};
use crate::board::{bishop_attacks, knight_attacks, pawn_attacks, rook_attacks};

use super::{MATE, MAX_PLY, SearchContext, quiescence::quiescence, tt::Bound};

// ---------------------------------------------------------------------------
// Singular extensions (Phase 3)
// ---------------------------------------------------------------------------

const SE_MIN_DEPTH: i32 = 8;
const SE_TT_DEPTH_SLACK: i32 = 3;
const SE_VERIF_RED: i32 = 3;
const SE_MARGIN_PER_DEPTH: i32 = 4;
const SE_EXT_CAP: u8 = 6;

// ---------------------------------------------------------------------------
// LMR table
// ---------------------------------------------------------------------------

static LMR_TABLE: OnceLock<[[u8; 64]; 64]> = OnceLock::new();

fn lmr_reduction(depth: i32, move_idx: usize) -> i32 {
    let table = LMR_TABLE.get_or_init(|| {
        let mut t = [[0u8; 64]; 64];
        for d in 0..64 {
            for m in 0..64 {
                if d < 3 || m < 3 {
                    t[d][m] = 0;
                } else {
                    let d_f = d as f64;
                    let m_f = m as f64;
                    let r = 0.77 + d_f.ln() * m_f.ln() / 2.36;
                    t[d][m] = (r as u8).min((d - 2) as u8);
                }
            }
        }
        t
    });
    let d = (depth as usize).min(63);
    let m = move_idx.min(63);
    table[d][m] as i32
}

#[inline]
#[allow(clippy::collapsible_if)]
fn is_quiet(pos: &Position, mv: Move) -> bool {
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

#[inline]
fn gives_check(pos: &Position, mv: Move) -> bool {
    // Bitboard gives-check without make/unmake: does the move attack the enemy king
    // with its post-move occupancy? Covers direct and discovered.
    let us = pos.side_to_move();
    let them = us.opposite();
    let ek = match pos.king_square(them) {
        Some(s) => s,
        None => return false,
    };
    let moving = match pos.piece_at(mv.from) {
        Some(p) => p,
        None => return false,
    };
    let occ = pos.occupied();
    let from_bb = Bitboard::from_sq(mv.from);
    let to_bb = Bitboard::from_sq(mv.to);
    let is_ep = moving.piece_type() == PieceType::Pawn && pos.en_passant() == Some(mv.to);
    let occ_after = if is_ep {
        let cap_sq = Square::from_coords(mv.to.file(), mv.from.rank());
        let cap_bb = Bitboard::from_sq(cap_sq);
        Bitboard((occ.0 ^ from_bb.0 ^ cap_bb.0) | to_bb.0)
    } else {
        Bitboard((occ.0 ^ from_bb.0) | to_bb.0)
    };

    // After bitboards for us attackers
    let mut us_pawns = pos.pieces_bb(Piece::new(us, PieceType::Pawn));
    let mut us_knights = pos.pieces_bb(Piece::new(us, PieceType::Knight));
    let mut us_bishops = pos.pieces_bb(Piece::new(us, PieceType::Bishop));
    let mut us_rooks = pos.pieces_bb(Piece::new(us, PieceType::Rook));
    let mut us_queens = pos.pieces_bb(Piece::new(us, PieceType::Queen));

    match moving.piece_type() {
        PieceType::Pawn => {
            us_pawns.0 &= !from_bb.0;
            if let Some(promo) = mv.promotion {
                match promo {
                    PieceType::Queen => us_queens.0 |= to_bb.0,
                    PieceType::Rook => us_rooks.0 |= to_bb.0,
                    PieceType::Bishop => us_bishops.0 |= to_bb.0,
                    PieceType::Knight => us_knights.0 |= to_bb.0,
                    PieceType::Pawn | PieceType::King => {}
                }
            } else {
                us_pawns.0 |= to_bb.0;
            }
        }
        PieceType::Knight => {
            us_knights.0 ^= from_bb.0 | to_bb.0;
        }
        PieceType::Bishop => {
            us_bishops.0 ^= from_bb.0 | to_bb.0;
        }
        PieceType::Rook => {
            us_rooks.0 ^= from_bb.0 | to_bb.0;
        }
        PieceType::Queen => {
            us_queens.0 ^= from_bb.0 | to_bb.0;
        }
        PieceType::King => {}
    }

    // Pawns attack
    if (pawn_attacks(ek, them) & us_pawns).0 != 0 {
        return true;
    }
    if (knight_attacks(ek) & us_knights).0 != 0 {
        return true;
    }
    // King cannot give check by moving adjacent (illegal), but discovered via other piece still counts — handled by sliders below.
    let bishops = us_bishops | us_queens;
    if (bishop_attacks(ek, occ_after) & bishops).0 != 0 {
        return true;
    }
    let rooks = us_rooks | us_queens;
    if (rook_attacks(ek, occ_after) & rooks).0 != 0 {
        return true;
    }
    false
}

/// Negamax alpha-beta.
///
/// `ply` is distance from root, `can_null` allows null-move pruning.
/// Updates `ctx.pv_table`/`pv_len` on PV nodes.
pub fn negamax(
    pos: &mut Position,
    mut depth: i32,
    mut alpha: i32,
    beta: i32,
    ply: usize,
    can_null: bool,
    excluded: Option<Move>,
    ctx: &mut SearchContext,
) -> i32 {
    let is_pv = beta > alpha + 1;

    if ctx.stop.load(Ordering::Relaxed) {
        return 0;
    }
    if ctx.tick_check() {
        ctx.flush_nodes();
        if ctx.tc.should_hard_stop() {
            ctx.stop.store(true, Ordering::Relaxed);
            return 0;
        }
        if let Some(limit) = ctx.limits.nodes {
            if ctx.total_nodes.load(Ordering::Relaxed) >= limit {
                ctx.stop.store(true, Ordering::Relaxed);
                return 0;
            }
        }
    }

    if ply >= MAX_PLY - 1 {
        return crate::eval::evaluate_cached(pos, &mut ctx.pawn_cache);
    }

    ctx.nodes += 1;
    if ply > ctx.seldepth {
        ctx.seldepth = ply;
    }
    ctx.pv_len[ply] = 0;

    if pos.is_repetition() || pos.is_fifty_move_draw() {
        return 0;
    }

    let mated = -MATE + ply as i32;
    // let mate = MATE - ply as i32 - 1; // beta capping is safe but not needed (exclusive bound)
    if alpha < mated {
        alpha = mated;
    }
    if alpha >= beta {
        return alpha;
    }

    // SE: initialize extension stack at root
    if ply == 0 {
        ctx.se_extensions[0] = 0;
    }
    let mut tt_move: Option<Move> = None;
    let mut tt_hit_for_se: Option<crate::search::tt::TtHit> = None;
    // When excluded is set, skip TT entirely (exclusion search uses same hash; storing would pollute)
    if excluded.is_none() {
        if let Some(hit) = ctx.tt.probe(pos.hash(), ply) {
            tt_move = hit.mv;
            tt_hit_for_se = Some(hit);
            if (hit.depth as i32) >= depth {
                match hit.bound {
                    Bound::Exact => return hit.score,
                    Bound::Lower if hit.score >= beta => return hit.score,
                    Bound::Upper if hit.score <= alpha => return hit.score,
                    _ => {}
                }
            }
        }
    }

    // IIR: no TT move at decent depth → search shallower to get a move
    // Do not apply IIR inside the exclusion search (tt_move is forced None there).
    if excluded.is_none() && tt_move.is_none() && depth >= 4 {
        depth -= 1;
    }

    let in_check = if let Some(king_sq) = pos.king_square(pos.side_to_move()) {
        is_square_attacked(pos, king_sq, pos.side_to_move().opposite())
    } else {
        false
    };
    if in_check {
        depth += 1;
    }

    if depth <= 0 {
        return quiescence(pos, alpha, beta, ply, ctx);
    }

    let static_eval = crate::eval::evaluate_cached(pos, &mut ctx.pawn_cache);

    // Singular extensions: verification search (Phase 3.2) — single ext, SF-style verif
    let mut singular_move: Option<Move> = None;
    let mut singular_ext: i32 = 0;
    if excluded.is_none()
        && tt_move.is_some()
        && depth >= SE_MIN_DEPTH
        && !ctx.stop.load(Ordering::Relaxed)
        && ply < MAX_PLY - 1
    {
        if let Some(hit) = tt_hit_for_se {
            let tt_depth_ok = (hit.depth as i32) >= depth - SE_TT_DEPTH_SLACK;
            let is_lower = hit.bound == Bound::Lower;
            let not_mate = hit.score.abs() < MATE - MAX_PLY as i32;
            if tt_depth_ok && is_lower && not_mate && ctx.se_extensions[ply] < SE_EXT_CAP {
                let margin = SE_MARGIN_PER_DEPTH * depth;
                let beta_se = hit.score - margin;
                let verif_depth = ((depth - 1) / 2).max(0);
                let se_score = negamax(
                    pos,
                    verif_depth,
                    beta_se - 1,
                    beta_se,
                    ply,
                    can_null,
                    tt_move,
                    ctx,
                );
                if ctx.stop.load(Ordering::Relaxed) {
                    return 0;
                }
                if se_score < beta_se {
                    singular_ext = 1;
                    singular_move = tt_move;
                }
            }
        }
    }

    // Razoring: shallow depth, eval far below alpha → qsearch verification
    if !is_pv && !in_check && depth <= 3 && static_eval.abs() < MATE - MAX_PLY as i32 {
        let razor_margin = 300 + 200 * (depth - 1);
        if static_eval + razor_margin < alpha {
            let score = quiescence(pos, alpha, beta, ply, ctx);
            if score < alpha {
                return score;
            }
        }
    }

    if !is_pv
        && !in_check
        && can_null
        && depth >= 3
        && static_eval >= beta
        && pos.has_non_pawn_material(pos.side_to_move())
    {
        let r = 2 + depth / 4;
        if ply < super::MAX_PLY {
            ctx.prev_stack[ply] = None;
        }
        pos.make_null_move();
        let score = -negamax(
            pos,
            depth - 1 - r,
            -beta,
            -beta + 1,
            ply + 1,
            false,
            None,
            ctx,
        );
        pos.unmake_null_move();
        if ctx.stop.load(Ordering::Relaxed) {
            return 0;
        }
        if score >= beta && score.abs() < MATE - MAX_PLY as i32 {
            return beta;
        }
    }

    // Generate legal moves into stack MoveList
    let mut list = MoveList::new();
    generate_legal_into(pos, &mut list);

    if list.is_empty() {
        if in_check {
            return -MATE + ply as i32;
        }
        return 0;
    }

    // Pre-score once (O(n) instead of O(n²)) — countermove + continuation history
    let counter = if ply > 0 {
        if let Some((pp, ps)) = ctx.prev_stack[ply - 1] {
            ctx.countermoves[pp as usize][ps.index() as usize]
        } else {
            None
        }
    } else {
        None
    };
    let prev1 = if ply > 0 {
        ctx.prev_stack[ply - 1]
    } else {
        None
    };
    let prev2 = if ply > 1 {
        ctx.prev_stack[ply - 2]
    } else {
        None
    };
    crate::search::order::score_list(
        pos,
        &mut list,
        tt_move,
        &ctx.killers[ply],
        counter,
        &ctx.history,
        prev1,
        &ctx.cont1,
        prev2,
        &ctx.cont2,
        ply,
    );

    ctx.pv_len[ply] = 0;

    let mut best_score = -MATE * 2;
    let original_alpha = alpha;
    let mut best_move: Option<Move> = None;
    // Tracker for history malus (10c): quiets actually searched at this node.
    // Stored per-ply in SearchContext (1.4) to avoid a ~1.3KB zeroed stack frame.
    let mut quiets_cnt: usize = 0;

    let list_len = list.len;
    for i in 0..list_len {
        let mv = list.pick_best(i);
        if Some(mv) == excluded {
            continue;
        }
        // Record prev_stack for child: piece type before move, to square (10a)
        if ply < super::MAX_PLY {
            if let Some(pc) = pos.piece_at(mv.from) {
                ctx.prev_stack[ply] = Some((pc.piece_type(), mv.to));
            } else {
                ctx.prev_stack[ply] = None;
            }
        }

        let quiet = is_quiet(pos, mv);

        if !is_pv && !in_check && quiet {
            if depth <= 6 && static_eval + 120 * depth <= alpha {
                continue;
            }
            if depth <= 4 && quiets_cnt >= 3 + (depth as usize * depth as usize) {
                continue;
            }
        }
        if quiet {
            // Safety: ply < MAX_PLY guaranteed by early return above
            ctx.quiets_stack[ply][quiets_cnt] = Some(mv);
            quiets_cnt += 1;
        }

        if ctx.stop.load(Ordering::Relaxed) {
            return 0;
        }

        // Singular extension for this move (Phase 3.2) — supports double
        let ext: i32 = if Some(mv) == singular_move {
            singular_ext
        } else {
            0
        };
        // Cap total extensions
        let ext = if ctx.se_extensions[ply].saturating_add(ext as u8) > SE_EXT_CAP {
            if SE_EXT_CAP > ctx.se_extensions[ply] {
                (SE_EXT_CAP - ctx.se_extensions[ply]) as i32
            } else {
                0
            }
        } else {
            ext
        };
        let next_se = ctx.se_extensions[ply].saturating_add(ext as u8);
        if ply + 1 < MAX_PLY {
            ctx.se_extensions[ply + 1] = next_se;
        }

        let mut reduction = 0;
        if ext == 0 && !is_pv && !in_check && quiet && depth >= 3 && i >= 3 && !gives_check(pos, mv) {
            reduction = lmr_reduction(depth, i);
            reduction = reduction.clamp(1, depth - 2);
            if reduction < 0 {
                reduction = 0;
            }
        }

        let score = if reduction > 0 {
            pos.make_move(mv);
            let v = -negamax(
                pos,
                depth - 1 + ext - reduction,
                -alpha - 1,
                -alpha,
                ply + 1,
                true,
                None,
                ctx,
            );
            pos.unmake_move(mv);
            if ctx.stop.load(Ordering::Relaxed) {
                return 0;
            }
            if v > alpha {
                pos.make_move(mv);
                let v2 = -negamax(
                    pos,
                    depth - 1 + ext,
                    -alpha - 1,
                    -alpha,
                    ply + 1,
                    true,
                    None,
                    ctx,
                );
                pos.unmake_move(mv);
                if ctx.stop.load(Ordering::Relaxed) {
                    return 0;
                }
                if v2 > alpha && is_pv {
                    pos.make_move(mv);
                    let v3 = -negamax(
                        pos,
                        depth - 1 + ext,
                        -beta,
                        -alpha,
                        ply + 1,
                        true,
                        None,
                        ctx,
                    );
                    pos.unmake_move(mv);
                    if ctx.stop.load(Ordering::Relaxed) {
                        return 0;
                    }
                    v3
                } else {
                    v2
                }
            } else {
                v
            }
        } else if i == 0 {
            pos.make_move(mv);
            let v = -negamax(
                pos,
                depth - 1 + ext,
                -beta,
                -alpha,
                ply + 1,
                true,
                None,
                ctx,
            );
            pos.unmake_move(mv);
            if ctx.stop.load(Ordering::Relaxed) {
                return 0;
            }
            v
        } else {
            pos.make_move(mv);
            let v = -negamax(
                pos,
                depth - 1 + ext,
                -alpha - 1,
                -alpha,
                ply + 1,
                true,
                None,
                ctx,
            );
            pos.unmake_move(mv);
            if ctx.stop.load(Ordering::Relaxed) {
                return 0;
            }
            if v > alpha {
                pos.make_move(mv);
                let v2 = -negamax(
                    pos,
                    depth - 1 + ext,
                    -beta,
                    -alpha,
                    ply + 1,
                    true,
                    None,
                    ctx,
                );
                pos.unmake_move(mv);
                if ctx.stop.load(Ordering::Relaxed) {
                    return 0;
                }
                v2
            } else {
                v
            }
        };

        if score > best_score {
            best_score = score;
            best_move = Some(mv);
        }

        if score > alpha {
            alpha = score;
            ctx.pv_table[ply][0] = Some(mv);
            let child_len = ctx.pv_len[ply + 1];
            for j in 0..child_len {
                ctx.pv_table[ply][j + 1] = ctx.pv_table[ply + 1][j];
            }
            ctx.pv_len[ply] = child_len + 1;

            if alpha >= beta {
                if quiet {
                    crate::search::order::update_killers(&mut ctx.killers, ply, mv);
                    let side = pos.side_to_move();
                    crate::search::order::update_history(&mut ctx.history, side, mv, depth);
                    // Countermove update (10a): store reply to prev move
                    if ply > 0 {
                        if let Some((pp, ps)) = ctx.prev_stack[ply - 1] {
                            ctx.countermoves[pp as usize][ps.index() as usize] = Some(mv);
                        }
                    }
                    // Continuation history updates (10b) + malus for non-cutoff quiets (10c)
                    let bonus = (depth * depth).clamp(0, 16384);
                    let malus = -(bonus / 2);
                    if let Some(pc) = pos.piece_at(mv.from) {
                        let cur_pt = pc.piece_type();
                        let cur_to = mv.to;
                        if ply > 0 {
                            if let Some((pp, ps)) = ctx.prev_stack[ply - 1] {
                                crate::search::order::update_cont(
                                    &mut ctx.cont1,
                                    pp,
                                    ps,
                                    cur_pt,
                                    cur_to,
                                    bonus,
                                );
                            }
                        }
                        if ply > 1 {
                            if let Some((pp, ps)) = ctx.prev_stack[ply - 2] {
                                crate::search::order::update_cont(
                                    &mut ctx.cont2,
                                    pp,
                                    ps,
                                    cur_pt,
                                    cur_to,
                                    bonus,
                                );
                            }
                        }
                    }
                    // Malus for quiets searched before the cutoff (10c)
                    for j in 0..quiets_cnt {
                        if let Some(qm) = ctx.quiets_stack[ply][j] {
                            if qm == mv {
                                continue;
                            }
                            // butterfly malus
                            crate::search::order::update_history_with_bonus(
                                &mut ctx.history,
                                side,
                                qm,
                                malus,
                            );
                            // cont malus (need qm's own cur piece/to)
                            if let Some(qpc) = pos.piece_at(qm.from) {
                                let q_pt = qpc.piece_type();
                                let q_to = qm.to;
                                if ply > 0 {
                                    if let Some((pp, ps)) = ctx.prev_stack[ply - 1] {
                                        crate::search::order::update_cont(
                                            &mut ctx.cont1,
                                            pp,
                                            ps,
                                            q_pt,
                                            q_to,
                                            malus,
                                        );
                                    }
                                }
                                if ply > 1 {
                                    if let Some((pp, ps)) = ctx.prev_stack[ply - 2] {
                                        crate::search::order::update_cont(
                                            &mut ctx.cont2,
                                            pp,
                                            ps,
                                            q_pt,
                                            q_to,
                                            malus,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                break;
            }
        }
    }

    let bound = if best_score <= original_alpha {
        Bound::Upper
    } else if best_score >= beta {
        Bound::Lower
    } else {
        Bound::Exact
    };
    let store_depth = depth.clamp(0, 127) as u8;
    if excluded.is_none() {
        ctx.tt.store(
            pos.hash(),
            best_move,
            best_score,
            static_eval,
            store_depth,
            bound,
            ply,
        );
    }

    if best_score > original_alpha {
        alpha
    } else {
        best_score
    }
}

#[cfg(test)]
mod gives_check_tests {
    use super::gives_check;
    use crate::board::{Move, Position, generate_legal, is_square_attacked};

    fn gives_check_slow(pos: &mut Position, mv: Move) -> bool {
        pos.make_move(mv);
        let opp = pos.side_to_move();
        let gives = if let Some(king_sq) = pos.king_square(opp) {
            is_square_attacked(pos, king_sq, opp.opposite())
        } else {
            false
        };
        pos.unmake_move(mv);
        gives
    }

    #[test]
    fn gives_check_matches_random() {
        use crate::board::fen::parse_fen;
        let fens = [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
            "r1bq1rk1/pp2ppbp/2np1np1/2p5/2P1P3/1PN1B3/PB1Q1PPP/R3K2R w KQ - 0 1",
        ];
        for fen in fens {
            let pos = parse_fen(fen).unwrap();
            let mut moves = Vec::new();
            generate_legal(&pos, &mut moves);
            for mv in moves {
                let fast = gives_check(&pos, mv);
                let mut pos2 = pos.clone();
                let slow = gives_check_slow(&mut pos2, mv);
                assert_eq!(
                    fast, slow,
                    "mismatch fen {} mv {} fast {} slow {}",
                    fen, mv, fast, slow
                );
            }
        }
        // random walk
        let mut pos =
            parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let mut rng: u64 = 0x9E3779B97F4A7C15;
        for _ in 0..200 {
            let mut moves = Vec::new();
            generate_legal(&pos, &mut moves);
            if moves.is_empty() {
                break;
            }
            for mv in &moves {
                let fast = gives_check(&pos, *mv);
                let mut pos2 = pos.clone();
                let slow = gives_check_slow(&mut pos2, *mv);
                assert_eq!(
                    fast,
                    slow,
                    "walk mismatch pos {} mv {} fast {} slow {}",
                    crate::board::fen::to_fen(&pos),
                    mv,
                    fast,
                    slow
                );
            }
            // pick pseudo random move
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let idx = (rng as usize) % moves.len();
            pos.make_move(moves[idx]);
            // small chance to undo? keep walk.
        }
    }
}

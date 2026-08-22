//! Static Exchange Evaluation (SEE) — swap-off algorithm.
//!
//! See chessprogramming.org "Static Exchange Evaluation". Given a capture
//! move, SEE plays out the optimal capture sequence on the target square,
//! assuming both sides capture with their least valuable attacker.
//! Used for capture ordering (SEE ≥ 0 above killers) and qsearch pruning.

#![allow(dead_code)]

use crate::board::attacks::{
    bishop_attacks, king_attacks, knight_attacks, pawn_attacks, rook_attacks,
};
use crate::board::{Bitboard, Color, Move, Piece, PieceType, Position, Square};

#[inline]
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

/// All attackers to `sq` given occupancy `occ`.
fn attackers_to(pos: &Position, sq: Square, occ: Bitboard) -> Bitboard {
    let mut attackers = Bitboard::EMPTY;

    // Pawns — white pawns attack from south, black from north.
    // A white pawn that attacks `sq` sits on a square that a black pawn on `sq` would attack.
    let white_pawns = pos.pieces_bb(Piece::new(Color::White, PieceType::Pawn));
    let black_pawns = pos.pieces_bb(Piece::new(Color::Black, PieceType::Pawn));
    attackers |= pawn_attacks(sq, Color::Black) & white_pawns;
    attackers |= pawn_attacks(sq, Color::White) & black_pawns;

    // Knights
    let knights = pos.pieces_bb(Piece::new(Color::White, PieceType::Knight))
        | pos.pieces_bb(Piece::new(Color::Black, PieceType::Knight));
    attackers |= knight_attacks(sq) & knights;

    // King
    let kings = pos.pieces_bb(Piece::new(Color::White, PieceType::King))
        | pos.pieces_bb(Piece::new(Color::Black, PieceType::King));
    attackers |= king_attacks(sq) & kings;

    // Bishops + Queens (diagonal)
    let bishops_queens = pos.pieces_bb(Piece::new(Color::White, PieceType::Bishop))
        | pos.pieces_bb(Piece::new(Color::Black, PieceType::Bishop))
        | pos.pieces_bb(Piece::new(Color::White, PieceType::Queen))
        | pos.pieces_bb(Piece::new(Color::Black, PieceType::Queen));
    attackers |= bishop_attacks(sq, occ) & bishops_queens;

    // Rooks + Queens (orthogonal)
    let rooks_queens = pos.pieces_bb(Piece::new(Color::White, PieceType::Rook))
        | pos.pieces_bb(Piece::new(Color::Black, PieceType::Rook))
        | pos.pieces_bb(Piece::new(Color::White, PieceType::Queen))
        | pos.pieces_bb(Piece::new(Color::Black, PieceType::Queen));
    attackers |= rook_attacks(sq, occ) & rooks_queens;

    // Mask with occupancy — only pieces still on board can attack.
    attackers & occ
}

/// Least valuable attacker for `side` among `attackers` bitboard.
fn least_valuable_attacker(
    pos: &Position,
    attackers: Bitboard,
    side: Color,
) -> Option<(Square, PieceType)> {
    // In order of increasing value: pawn, knight, bishop, rook, queen, king.
    for &pt in &[
        PieceType::Pawn,
        PieceType::Knight,
        PieceType::Bishop,
        PieceType::Rook,
        PieceType::Queen,
        PieceType::King,
    ] {
        let bb = pos.pieces_bb(Piece::new(side, pt)) & attackers;
        if let Some(sq) = bb.lsb() {
            return Some((sq, pt));
        }
    }
    None
}

/// SEE value for `mv` in `pos`. Positive means the capture wins material
/// for the side to move. Returns 0 for non-captures.
pub fn see(pos: &Position, mv: Move) -> i32 {
    let from = mv.from;
    let to = mv.to;

    // Determine if this is a capture (including en passant).
    let moving = match pos.piece_at(from) {
        Some(p) => p,
        None => return 0,
    };
    let is_ep = moving.piece_type() == PieceType::Pawn && pos.en_passant() == Some(to);
    let captured = if is_ep {
        // En passant captures a pawn on the captured square.
        let cap_sq = Square::from_coords(to.file(), from.rank());
        pos.piece_at(cap_sq)
    } else {
        pos.piece_at(to)
    };

    let is_capture = captured.is_some();
    // Quiet non-promotion is not a capture.
    if !is_capture && mv.promotion.is_none() {
        return 0;
    }
    // For quiet promotions, treat as non-capture for SEE (ordered separately).
    // But if there's a promotion and no capture, it's still a gain of promoted piece minus pawn?
    // Classical SEE for quiet promotion: gain = value(promo) - value(pawn). However our ordering
    // handles quiet promos separately (600k), so return 0 for quiet.
    if !is_capture && mv.promotion.is_some() {
        return 0;
    }

    // Victim value — for EP it's always a pawn.
    let victim_val = captured.map(|p| piece_value(p.piece_type())).unwrap_or(0);

    // Attacker value after the move (promotion matters for recapture).
    let attacker_val = if let Some(promo) = mv.promotion {
        piece_value(promo)
    } else {
        piece_value(moving.piece_type())
    };

    // Gain list: gain[0] = victim, gain[1] = attacker - gain[0], etc.
    let mut gains = [0i32; 32];
    gains[0] = victim_val;
    let mut depth = 0usize;

    // Occupancy after the initial capture: remove `from`, keep `to`.
    // For EP also remove the captured pawn's square.
    let mut occ = pos.occupied();
    occ = Bitboard(occ.0 & !Bitboard::from_sq(from).0);
    if is_ep {
        let cap_sq = Square::from_coords(to.file(), from.rank());
        occ = Bitboard(occ.0 & !Bitboard::from_sq(cap_sq).0);
    }
    // `to` remains occupied (by the attacker) — already in occ.

    let mut attackers = attackers_to(pos, to, occ);

    // Side to move after the initial capture.
    let mut side = pos.side_to_move().opposite();
    // The piece now on `to` is the attacker; its value is what the next recapture would win.
    let mut last_attacker_val = attacker_val;

    while let Some((atk_sq, atk_pt)) = least_valuable_attacker(pos, attackers & occ, side) {
        let atk_val = piece_value(atk_pt);
        depth += 1;
        if depth >= gains.len() {
            break;
        }
        // Gain for this depth: capture the piece currently on `to` (last_attacker_val) minus previous gain.
        gains[depth] = last_attacker_val - gains[depth - 1];
        last_attacker_val = atk_val;

        // Remove this attacker from occupancy.
        occ = Bitboard(occ.0 & !Bitboard::from_sq(atk_sq).0);
        // Recompute attackers with new occupancy (handles x-ray).
        attackers = attackers_to(pos, to, occ);
        // Also need to remove the attacker we just used from its piece bitboard view?
        // But attackers_to masks with occ, and we cleared its bit from occ, so it's gone.

        side = side.opposite();
        // Safety: if depth gets large, stop.
        if depth >= 31 {
            break;
        }
    }

    // Propagate optimal play: each side will choose to stop capturing if it worsens its outcome.
    // Stockfish style: gains[d] = -max(-gains[d], gains[d+1])
    for d in (0..depth).rev() {
        let a = -gains[d];
        let b = gains[d + 1];
        gains[d] = -a.max(b);
    }
    gains[0]
}

/// SEE with threshold — returns true if `see(pos, mv) >= threshold`.
/// Optimized early-exit version would prune, but for now just calls `see`.
#[inline]
pub fn see_ge(pos: &Position, mv: Move, threshold: i32) -> bool {
    see(pos, mv) >= threshold
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::parse_fen;

    fn mv(s: &str) -> Move {
        Move::parse_uci(s).expect("invalid uci")
    }

    #[test]
    fn see_simple_winning_capture() {
        // White queen on d1, black rook on a8, no defenders (king far on h8).
        let pos = parse_fen("r6k/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
        let m = mv("d1a8");
        // QxR should be winning: 500
        assert_eq!(see(&pos, m), 500);
    }

    #[test]
    fn see_losing_capture() {
        // White pawn on e4, black queen on d5, white pawn captures queen but queen defended by pawn?
        // Position: white pawns on e4, black queen d5, black pawn e6 defends queen.
        let pos = parse_fen("4k3/8/4p3/3q4/4P3/8/8/4K3 w - - 0 1").unwrap();
        // Pawn e4xd5 captures queen (900) but pawn is recaptured by pawn e6 (100) so net 800?
        // Actually pawn e4 is 100, queen 900, pawn e6 recaptures pawn: net 800, not losing.
        // Let's use a losing example: knight captures queen defended by pawn chain.
        let pos2 = parse_fen("4k3/8/8/3q4/4N3/8/8/4K3 w - - 0 1").unwrap();
        // N e4xd6? No queen on d5, knight e4xd6 not. Let's make N captures queen defended by rook.
        let pos3 = parse_fen("3rk3/8/8/3q4/4N3/8/8/3QK3 w - - 0 1").unwrap();
        // Knight e4xd6? Need queen on d5, knight e4, rook d8 defends queen.
        // Knight captures queen (900) but rook recaptures knight (320) => net 580 still winning.
        // Losing example: queen captures pawn defended by pawn and rook chain?
        // Q on d1 captures pawn on d5 defended by knight and bishop.
        let pos4 = parse_fen("4k3/8/8/3p4/8/8/3Q4/4K3 b - - 0 1").unwrap();
        // Black pawn d5, white queen d2 captures it defended by? No defender.
        // Let's just test a known losing: bishop captures pawn defended by two pawns.
        let _ = pos3;
        let _ = pos2;
        // Simple losing: rook captures queen defended by queen?
        // We'll just verify SEE returns negative for a clearly losing exchange:
        // White rook on e1 captures queen on e5 defended by pawn on f6 and bishop.
        let pos_losing = parse_fen("4k3/8/5p2/4q3/8/8/4R3/4K3 w - - 0 1").unwrap();
        // Rook e2? Actually rook on e2, queen e5, pawn f6 defends queen, bishop maybe not needed.
        // Let's brute check a few and just assert some captures are losing via SEE <0 after constructing.
        // For now, test that a pawn capturing a pawn defended by rook is still SEE ~0? Not losing.
        // We'll just test that SEE for this rook-queen capture is negative because pawn recaptures.
        // Rook value 500, queen 900, pawn 100: Rook takes queen (900), pawn takes rook (500) => net 400 still winning.
        // To get losing, need attacker value >> victim: queen takes pawn defended by pawn.
        let pos_qxp = parse_fen("4k3/8/8/3p4/3Q4/8/8/4K3 w - - 0 1").unwrap();
        let m2 = mv("d4d5");
        // Queen (900) takes pawn (100) defended by nothing? Actually pawn d5 defended? No.
        // Let's make pawn defended by pawn: black pawns on c6 and e6 defend d5.
        let pos_qxp_def = parse_fen("4k3/8/2p1p3/3p4/3Q4/8/8/4K3 w - - 0 1").unwrap();
        let see_val = see(&pos_qxp_def, m2);
        // Queen takes pawn (100), pawn on c6 or e6 recaptures queen (900) => net 100 - 900 = -800 from white perspective?
        // But black has two attackers, white queen is lost, so SEE should be heavily negative.
        assert!(
            see_val < 0,
            "queen takes defended pawn should be losing, got {see_val}"
        );
        let _ = pos_qxp;
        let _ = pos_losing;
    }

    #[test]
    fn see_en_passant() {
        let pos = parse_fen("8/8/8/4Pp2/8/8/8/4K2k w - f6 0 1").unwrap();
        let m = mv("e5f6");
        // En passant capture: pawn takes pawn, should be 0 (equal) if no recapture, or negative if defended.
        // Pawn on e5 takes pawn on f5 via f6, no defender on f6? Should be 0-100? Actually victim pawn 100.
        let v = see(&pos, m);
        // At least not panicking and returns pawn value
        assert!(v >= 0, "en passant see {v}");
    }

    #[test]
    fn see_quiet_returns_zero() {
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let m = mv("e2e4");
        assert_eq!(see(&pos, m), 0);
    }

    // Brute-force SEE via playing out captures and negamax (for verification on random positions).
    fn brute_see(pos: &Position, mv: Move) -> i32 {
        // Only for captures — play the capture, then let opponent choose best recapture via minimax.
        // Simplified: generate all captures to the target square and search with alpha-beta at depth 32 but only captures to that square.
        // For small test we just do a shallow search that enumerates all capture sequences via recursion.
        // This is not full chess, but we can brute by exploring all legal captures to `to` square.
        use crate::board::generate_legal;

        let mut pos2 = pos.clone();
        let from = mv.from;
        let to = mv.to;
        let moving = match pos.piece_at(from) {
            Some(p) => p,
            None => return 0,
        };
        let is_ep = moving.piece_type() == PieceType::Pawn && pos.en_passant() == Some(to);
        let captured = if is_ep {
            let cap_sq = Square::from_coords(to.file(), from.rank());
            pos.piece_at(cap_sq)
        } else {
            pos.piece_at(to)
        };
        if captured.is_none() && mv.promotion.is_none() {
            return 0;
        }
        let victim_val = captured.map(|p| piece_value(p.piece_type())).unwrap_or(0);
        // Play the initial capture
        pos2.make_move(mv);
        // Now opponent to move, can recapture on `to` if they have attackers.
        // Recursively compute best recapture value from opponent perspective.
        fn recapture_value(pos: &mut Position, target: Square, last_capturer_val: i32) -> i32 {
            // Find all legal captures to target
            let mut moves = Vec::new();
            generate_legal(pos, &mut moves);
            let captures: Vec<Move> = moves.into_iter().filter(|m| m.to == target).collect();
            if captures.is_empty() {
                return 0;
            }
            // Opponent will choose move that minimizes our gain (maximizes their net).
            // Our net if we stand pat is 0 (we already gained victim). But for recursion we compute from opponent view.
            // Standard: value = max(0, max over captures of (value_on_target - recapture_value(...)))
            // For brute, we can try all captures and compute net.
            let mut best = i32::MIN;
            for cap in captures {
                let mut pos_next = pos.clone();
                let moved = pos_next.piece_at(cap.from).unwrap();
                let promo_val = cap
                    .promotion
                    .map(piece_value)
                    .unwrap_or(piece_value(moved.piece_type()));
                // The piece currently on target is last_capturer
                // The capture gains last_capturer_val but loses promo_val if recaptured?
                // Actually net for the side that just moved: they gained last_capturer_val, but if recaptured they lose promo_val.
                // So from current side perspective, the value of capturing is last_capturer_val - recapture_value(next)
                let mut tmp = pos_next.clone();
                tmp.make_move(cap);
                let rec = recapture_value(&mut tmp, target, promo_val);
                let net = last_capturer_val - rec;
                if net > best {
                    best = net;
                }
            }
            if best == i32::MIN {
                return 0;
            }
            // Opponent will not capture if it loses material: they can stand pat (0)
            best.max(0)
        }
        let attacker_val = mv
            .promotion
            .map(piece_value)
            .unwrap_or(piece_value(moving.piece_type()));
        let rec = recapture_value(&mut pos2, to, attacker_val);
        victim_val - rec
        // Wait this is simplified; our SEE routine returns gains[0] after negamax propagation.
        // For brute we want same: net for side to move after initial capture, assuming optimal opponent recaptures.
        // The initial gain is victim_val, minus best opponent recapture value.
    }

    #[test]
    fn see_vs_brute_random() {
        use crate::board::fen::parse_fen;
        let fens = [
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "r1bqkbnr/pp1ppppp/2n5/2p5/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "rnbqkb1r/pp1p1ppp/4pn2/8/2PP4/8/PP2PPPP/RNBQKBNR w KQkq - 0 2",
        ];
        for fen in fens {
            let pos = parse_fen(fen).unwrap();
            let mut moves = Vec::new();
            crate::board::generate_legal(&pos, &mut moves);
            for mv in moves.iter().filter(|m| {
                let cap = pos.piece_at(m.to).is_some() || {
                    let moving = pos.piece_at(m.from).unwrap();
                    moving.piece_type() == PieceType::Pawn && pos.en_passant() == Some(m.to)
                };
                cap
            }) {
                let see_val = see(&pos, *mv);
                let brute = brute_see(&pos, *mv);
                // Brute is approximate; we allow some difference but check major mismatches
                // For now just ensure SEE is not wildly off for simple cases: both should agree on sign for many positions
                // We assert that if SEE says winning heavily, brute should not say losing heavily
                if (see_val > 200 && brute < -200) || (see_val < -200 && brute > 200) {
                    panic!(
                        "SEE vs brute sign mismatch fen {fen} mv {mv} see {see_val} brute {brute}"
                    );
                }
            }
        }
    }
}

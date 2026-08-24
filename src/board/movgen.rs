#![allow(clippy::collapsible_if)]
//! Move generation — legal moves via classical checkers/pin masks.
//!
//! Legal moves are generated directly via pin-mask / checkers — the
//! pseudo-legal + make/unmake filter is gone.  See chessprogramming.org "Legal Move
//! Generation — Classical (Checkers and Pin Masks)" and "En Passant".
//!
//! The generator is pure: it never mutates the position and performs no
//! make/unmake.  `is_square_attacked` is the only geometry primitive.

use crate::board::{
    attacks::{
        bishop_attacks, king_attacks, knight_attacks, pawn_attacks, queen_attacks, rook_attacks,
    },
    movelist::MoveList,
    mv::Move,
    position::{CASTLE_BK, CASTLE_BQ, CASTLE_WK, CASTLE_WQ, Position},
    types::{A1, A8, Bitboard, Color, E4, H1, H8, Piece, PieceType, Square},
};

// ---------------------------------------------------------------------------
// Helpers: attack detection with custom occupancy
// ---------------------------------------------------------------------------

#[inline]
fn is_square_attacked_with_occ(pos: &Position, sq: Square, attacker: Color, occ: Bitboard) -> bool {
    // Pawns — pawn_attacks(sq, defender) & attacker pawns
    let pawns = pos.pieces_bb(Piece::new(attacker, PieceType::Pawn));
    if (pawn_attacks(sq, attacker.opposite()) & pawns).0 != 0 {
        return true;
    }
    let knights = pos.pieces_bb(Piece::new(attacker, PieceType::Knight));
    if (knight_attacks(sq) & knights).0 != 0 {
        return true;
    }
    let king = pos.pieces_bb(Piece::new(attacker, PieceType::King));
    if (king_attacks(sq) & king).0 != 0 {
        return true;
    }
    let bishops = pos.pieces_bb(Piece::new(attacker, PieceType::Bishop))
        | pos.pieces_bb(Piece::new(attacker, PieceType::Queen));
    if (bishop_attacks(sq, occ) & bishops).0 != 0 {
        return true;
    }
    let rooks = pos.pieces_bb(Piece::new(attacker, PieceType::Rook))
        | pos.pieces_bb(Piece::new(attacker, PieceType::Queen));
    if (rook_attacks(sq, occ) & rooks).0 != 0 {
        return true;
    }
    false
}

/// Check if `sq` is attacked by `attacker` color in `pos` (current occupancy).
pub fn is_square_attacked(pos: &Position, sq: Square, attacker: Color) -> bool {
    is_square_attacked_with_occ(pos, sq, attacker, pos.occupied())
}

// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

#[inline]
fn squares_between(a: Square, b: Square) -> Bitboard {
    let af = a.file() as i8;
    let ar = a.rank() as i8;
    let bf = b.file() as i8;
    let br = b.rank() as i8;
    let df = bf - af;
    let dr = br - ar;
    if df == 0 && dr == 0 {
        return Bitboard::EMPTY;
    }
    if df != 0 && dr != 0 && df.abs() != dr.abs() {
        return Bitboard::EMPTY;
    }
    let step_f = df.signum();
    let step_r = dr.signum();
    let mut bb = Bitboard::EMPTY;
    let mut f = af + step_f;
    let mut r = ar + step_r;
    while f != bf || r != br {
        if !(0..8).contains(&f) || !(0..8).contains(&r) {
            break;
        }
        bb |= Bitboard::from_sq(Square::from_coords(f as u8, r as u8));
        f += step_f;
        r += step_r;
    }
    bb
}

fn get_checkers(pos: &Position, king_sq: Square, them: Color, occ: Bitboard) -> Bitboard {
    let mut checkers = Bitboard::EMPTY;
    let us = them.opposite();
    // Pawns
    let them_pawns = pos.pieces_bb(Piece::new(them, PieceType::Pawn));
    checkers |= pawn_attacks(king_sq, us) & them_pawns;
    // Knights
    let them_knights = pos.pieces_bb(Piece::new(them, PieceType::Knight));
    checkers |= knight_attacks(king_sq) & them_knights;
    // King (adjacent) — normally unreachable in legal positions
    // let them_king = pos.pieces_bb(Piece::new(them, PieceType::King));
    // checkers |= king_attacks(king_sq) & them_king;
    // Bishops / queens
    let them_bq = pos.pieces_bb(Piece::new(them, PieceType::Bishop))
        | pos.pieces_bb(Piece::new(them, PieceType::Queen));
    checkers |= bishop_attacks(king_sq, occ) & them_bq;
    // Rooks / queens
    let them_rq = pos.pieces_bb(Piece::new(them, PieceType::Rook))
        | pos.pieces_bb(Piece::new(them, PieceType::Queen));
    checkers |= rook_attacks(king_sq, occ) & them_rq;
    checkers
}

fn get_pinned(
    pos: &Position,
    king_sq: Square,
    us: Color,
    them: Color,
    occ: Bitboard,
) -> (Bitboard, [Bitboard; 64]) {
    let us_occ = pos.occupied_color(us);
    let mut pinned = Bitboard::EMPTY;
    let mut pin_rays = [Bitboard::EMPTY; 64];
    let dirs: [(i8, i8); 8] = [
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (1, -1),
        (-1, 1),
        (-1, -1),
    ];
    for (df, dr) in dirs {
        let mut first: Option<Square> = None;
        let mut second: Option<Square> = None;
        let mut f = king_sq.file() as i8 + df;
        let mut r = king_sq.rank() as i8 + dr;
        while (0..8).contains(&f) && (0..8).contains(&r) {
            let sq = Square::from_coords(f as u8, r as u8);
            if occ.contains(sq) {
                if first.is_none() {
                    first = Some(sq);
                } else {
                    second = Some(sq);
                    break;
                }
            }
            f += df;
            r += dr;
        }
        if let (Some(fst), Some(snd)) = (first, second) {
            if !us_occ.contains(fst) {
                continue;
            }
            let p = match pos.piece_at(snd) {
                Some(pc) => pc,
                None => continue,
            };
            if p.color() != them {
                continue;
            }
            let pt = p.piece_type();
            let is_diag = df != 0 && dr != 0;
            let is_orth = df == 0 || dr == 0;
            let slider = match pt {
                PieceType::Bishop => is_diag,
                PieceType::Rook => is_orth,
                PieceType::Queen => true,
                _ => false,
            };
            if !slider {
                continue;
            }
            pinned |= Bitboard::from_sq(fst);
            // build ray from king to snd inclusive (excluding king)
            let mut ray = Bitboard::EMPTY;
            let mut rf = king_sq.file() as i8 + df;
            let mut rr = king_sq.rank() as i8 + dr;
            while (0..8).contains(&rf) && (0..8).contains(&rr) {
                let rsq = Square::from_coords(rf as u8, rr as u8);
                ray |= Bitboard::from_sq(rsq);
                if rsq == snd {
                    break;
                }
                rf += df;
                rr += dr;
            }
            pin_rays[fst.index() as usize] = ray;
        }
    }
    (pinned, pin_rays)
}

// ---------------------------------------------------------------------------
// Castling helpers (const squares to avoid from_str in hot path)
// ---------------------------------------------------------------------------

const E1: Square = Square(4);
const F1: Square = Square(5);
const G1: Square = Square(6);
const H1_SQ: Square = Square(7);
const D1: Square = Square(3);
const C1: Square = Square(2);
const B1: Square = Square(1);
const A1_SQ: Square = Square(0);
const E8: Square = Square(60);
const F8: Square = Square(61);
const G8: Square = Square(62);
const H8_SQ: Square = Square(63);
const D8: Square = Square(59);
const C8: Square = Square(58);
const B8: Square = Square(57);
const A8_SQ: Square = Square(56);

// ---------------------------------------------------------------------------
// Public generators — &Position API (call sites may pass &mut, coerces)
// ---------------------------------------------------------------------------

/// Generate all legal moves.
pub fn generate_legal(pos: &Position, moves: &mut Vec<Move>) {
    let us = pos.side_to_move();
    let them = us.opposite();
    let occ = pos.occupied();
    let us_occ = pos.occupied_color(us);
    let them_occ = pos.occupied_color(them);

    let king_sq = match pos.king_square(us) {
        Some(s) => s,
        None => return,
    };

    let checkers = get_checkers(pos, king_sq, them, occ);
    let check_count = checkers.count() as usize;
    let (pinned, pin_rays) = get_pinned(pos, king_sq, us, them, occ);

    // King moves (always)
    {
        let king_bb = Bitboard::from_sq(king_sq);
        let occ_without_king = Bitboard(occ.0 ^ king_bb.0);
        let king_moves = king_attacks(king_sq) & !us_occ;
        for to in king_moves.squares() {
            if is_square_attacked_with_occ(pos, to, them, occ_without_king) {
                continue;
            }
            moves.push(Move::new(king_sq, to, None));
        }
    }

    if check_count >= 2 {
        // Double check — only king moves
        return;
    }

    let check_ray = if check_count == 1 {
        let checker_sq = checkers.lsb().unwrap();
        squares_between(king_sq, checker_sq) | checkers
    } else {
        Bitboard::ALL
    };

    // --- Pawns ---
    let pawns = pos.pieces_bb(Piece::new(us, PieceType::Pawn));
    let dir: i8 = if us == Color::White { 1 } else { -1 };
    let start_rank: i8 = if us == Color::White { 1 } else { 6 };
    let promo_rank: u8 = if us == Color::White { 7 } else { 0 };
    let ep_sq = pos.en_passant();

    for from in pawns.squares() {
        let is_pinned = pinned.contains(from);
        let pin_ray = pin_rays[from.index() as usize];
        let fr = from.rank() as i8;
        let ff = from.file() as i8;

        // Single push
        let nr = fr + dir;
        if (0..8).contains(&nr) {
            let to = Square::from_coords(ff as u8, nr as u8);
            if !occ.contains(to) {
                let pinned_ok = !is_pinned || pin_ray.contains(to);
                let check_ok = check_count != 1 || check_ray.contains(to);
                if pinned_ok && check_ok {
                    if nr as u8 == promo_rank {
                        for promo in [
                            PieceType::Queen,
                            PieceType::Rook,
                            PieceType::Bishop,
                            PieceType::Knight,
                        ] {
                            moves.push(Move::new(from, to, Some(promo)));
                        }
                    } else {
                        moves.push(Move::new(from, to, None));
                    }
                }
                // Double push — independent of single's check/pin for the single square;
                // the double's destination must be on the pin ray / check ray in its own right.
                if fr == start_rank {
                    let nr2 = nr + dir;
                    if (0..8).contains(&nr2) {
                        let to2 = Square::from_coords(ff as u8, nr2 as u8);
                        if !occ.contains(to2) {
                            let pinned_ok2 = !is_pinned || pin_ray.contains(to2);
                            let check_ok2 = check_count != 1 || check_ray.contains(to2);
                            if pinned_ok2 && check_ok2 {
                                moves.push(Move::new(from, to2, None));
                            }
                        }
                    }
                }
            }
        }

        // Captures (including EP and promo captures)
        for df in [-1_i8, 1] {
            let nf = ff + df;
            let nr2 = fr + dir;
            if !(0..8).contains(&nf) || !(0..8).contains(&nr2) {
                continue;
            }
            let to = Square::from_coords(nf as u8, nr2 as u8);
            let is_ep = ep_sq == Some(to);
            if is_ep {
                // EP — cap square is behind to
                let cap_sq = Square::from_coords(nf as u8, fr as u8);
                // Must be enemy pawn on cap
                let cap_piece = pos.piece_at(cap_sq);
                if cap_piece != Some(Piece::new(them, PieceType::Pawn)) {
                    continue;
                }
                if is_pinned && !pin_ray.contains(to) {
                    continue;
                }
                if check_count == 1 {
                    let checker_sq = checkers.lsb().unwrap();
                    if cap_sq != checker_sq {
                        continue;
                    }
                }
                // Rank-discovered check via the two pawns leaving the rank
                let mut occ_ep = occ;
                occ_ep.0 ^= Bitboard::from_sq(from).0;
                occ_ep.0 ^= Bitboard::from_sq(cap_sq).0;
                occ_ep.0 |= Bitboard::from_sq(to).0;
                if is_square_attacked_with_occ(pos, king_sq, them, occ_ep) {
                    continue;
                }
                moves.push(Move::new(from, to, None));
            } else {
                // Normal capture
                if !them_occ.contains(to) {
                    continue;
                }
                if is_pinned && !pin_ray.contains(to) {
                    continue;
                }
                if check_count == 1 && !check_ray.contains(to) {
                    continue;
                }
                if nr2 as u8 == promo_rank {
                    for promo in [
                        PieceType::Queen,
                        PieceType::Rook,
                        PieceType::Bishop,
                        PieceType::Knight,
                    ] {
                        moves.push(Move::new(from, to, Some(promo)));
                    }
                } else {
                    moves.push(Move::new(from, to, None));
                }
            }
        }
    }

    // --- Knights ---
    let knights = pos.pieces_bb(Piece::new(us, PieceType::Knight));
    for from in knights.squares() {
        if pinned.contains(from) {
            continue;
        }
        let mut attacks = knight_attacks(from) & !us_occ;
        if check_count == 1 {
            attacks &= check_ray;
        }
        for to in attacks.squares() {
            moves.push(Move::new(from, to, None));
        }
    }

    // --- Bishops ---
    let bishops = pos.pieces_bb(Piece::new(us, PieceType::Bishop));
    for from in bishops.squares() {
        let mut attacks = bishop_attacks(from, occ) & !us_occ;
        if pinned.contains(from) {
            attacks &= pin_rays[from.index() as usize];
        }
        if check_count == 1 {
            attacks &= check_ray;
        }
        for to in attacks.squares() {
            moves.push(Move::new(from, to, None));
        }
    }

    // --- Rooks ---
    let rooks = pos.pieces_bb(Piece::new(us, PieceType::Rook));
    for from in rooks.squares() {
        let mut attacks = rook_attacks(from, occ) & !us_occ;
        if pinned.contains(from) {
            attacks &= pin_rays[from.index() as usize];
        }
        if check_count == 1 {
            attacks &= check_ray;
        }
        for to in attacks.squares() {
            moves.push(Move::new(from, to, None));
        }
    }

    // --- Queens ---
    let queens = pos.pieces_bb(Piece::new(us, PieceType::Queen));
    for from in queens.squares() {
        let mut attacks = queen_attacks(from, occ) & !us_occ;
        if pinned.contains(from) {
            attacks &= pin_rays[from.index() as usize];
        }
        if check_count == 1 {
            attacks &= check_ray;
        }
        for to in attacks.squares() {
            moves.push(Move::new(from, to, None));
        }
    }

    // --- Castling (only when not in check) ---
    if check_count != 0 {
        return;
    }
    if us == Color::White {
        if pos.castling() & CASTLE_WK != 0 {
            if pos.piece_at(F1).is_none()
                && pos.piece_at(G1).is_none()
                && pos.piece_at(H1_SQ) == Some(Piece::WR)
                && !is_square_attacked(pos, E1, them)
                && !is_square_attacked(pos, F1, them)
                && !is_square_attacked(pos, G1, them)
            {
                moves.push(Move::new(E1, G1, None));
            }
        }
        if pos.castling() & CASTLE_WQ != 0 {
            if pos.piece_at(D1).is_none()
                && pos.piece_at(C1).is_none()
                && pos.piece_at(B1).is_none()
                && pos.piece_at(A1_SQ) == Some(Piece::WR)
                && !is_square_attacked(pos, E1, them)
                && !is_square_attacked(pos, D1, them)
                && !is_square_attacked(pos, C1, them)
            {
                moves.push(Move::new(E1, C1, None));
            }
        }
    } else {
        if pos.castling() & CASTLE_BK != 0 {
            if pos.piece_at(F8).is_none()
                && pos.piece_at(G8).is_none()
                && pos.piece_at(H8_SQ) == Some(Piece::BR)
                && !is_square_attacked(pos, E8, them)
                && !is_square_attacked(pos, F8, them)
                && !is_square_attacked(pos, G8, them)
            {
                moves.push(Move::new(E8, G8, None));
            }
        }
        if pos.castling() & CASTLE_BQ != 0 {
            if pos.piece_at(D8).is_none()
                && pos.piece_at(C8).is_none()
                && pos.piece_at(B8).is_none()
                && pos.piece_at(A8_SQ) == Some(Piece::BR)
                && !is_square_attacked(pos, E8, them)
                && !is_square_attacked(pos, D8, them)
                && !is_square_attacked(pos, C8, them)
            {
                moves.push(Move::new(E8, C8, None));
            }
        }
    }
}

/// Generate pseudo-legal moves (legacy wrapper — kept for compat, not used in search).
/// Now just forwards to the legal generator; behaviour may differ from the
/// historic pseudo-legal set in corner cases but keeps the symbol alive for tests.
pub fn generate_pseudo_legal(pos: &Position, moves: &mut Vec<Move>) {
    generate_legal(pos, moves);
}

/// Generate pseudo-legal captures and promotions (legacy wrapper).
pub fn generate_pseudo_captures(pos: &Position, moves: &mut Vec<Move>) {
    generate_captures(pos, moves);
}

/// Generate legal captures and promotions (for quiescence).
///
/// Includes pawn captures (inc. EP), promotion pushes/captures, and all piece
/// captures. Castling and quiet pawn pushes are excluded. When in check the
/// only moves that can resolve the check are generated (and an EP that captures
/// the checking pawn is included even though its destination differs from the
/// checker square — the rank-pin occupancy test still applies).
pub fn generate_captures(pos: &Position, moves: &mut Vec<Move>) {
    let us = pos.side_to_move();
    let them = us.opposite();
    let occ = pos.occupied();
    let them_occ = pos.occupied_color(them);

    let king_sq = match pos.king_square(us) {
        Some(s) => s,
        None => return,
    };

    let checkers = get_checkers(pos, king_sq, them, occ);
    let check_count = checkers.count() as usize;
    let (pinned, pin_rays) = get_pinned(pos, king_sq, us, them, occ);

    // King captures
    {
        let king_bb = Bitboard::from_sq(king_sq);
        let occ_without_king = Bitboard(occ.0 ^ king_bb.0);
        let king_caps = king_attacks(king_sq) & them_occ;
        for to in king_caps.squares() {
            if is_square_attacked_with_occ(pos, to, them, occ_without_king) {
                continue;
            }
            // If double check, king captures are the only legal captures — still filtered by check_ray for single check
            if check_count == 1 {
                let checker_sq = checkers.lsb().unwrap();
                let ray = squares_between(king_sq, checker_sq) | checkers;
                if !ray.contains(to) {
                    // King captures that capture the checker are on ray; other king captures are evasions — allowed even when not on ray.
                    // In check, king may move anywhere not attacked — not just the ray. So skip the ray filter for king.
                }
            }
            moves.push(Move::new(king_sq, to, None));
        }
        if check_count >= 2 {
            return;
        }
    }

    if check_count == 1 {
        // Non-king captures must be on the checker or be an EP that removes the checker.
        // handled per piece below via check_ray.
    }

    let check_ray = if check_count == 1 {
        let checker_sq = checkers.lsb().unwrap();
        squares_between(king_sq, checker_sq) | checkers
    } else {
        Bitboard::ALL
    };

    // Pawns — captures + promo pushes
    let pawns = pos.pieces_bb(Piece::new(us, PieceType::Pawn));
    let dir: i8 = if us == Color::White { 1 } else { -1 };
    let promo_rank: u8 = if us == Color::White { 7 } else { 0 };
    let ep_sq = pos.en_passant();

    for from in pawns.squares() {
        let is_pinned = pinned.contains(from);
        let pin_ray = pin_rays[from.index() as usize];
        let fr = from.rank() as i8;
        let ff = from.file() as i8;
        // Captures
        for df in [-1_i8, 1] {
            let nf = ff + df;
            let nr = fr + dir;
            if !(0..8).contains(&nf) || !(0..8).contains(&nr) {
                continue;
            }
            let to = Square::from_coords(nf as u8, nr as u8);
            let is_ep = ep_sq == Some(to);
            if is_ep {
                let cap_sq = Square::from_coords(nf as u8, fr as u8);
                if pos.piece_at(cap_sq) != Some(Piece::new(them, PieceType::Pawn)) {
                    continue;
                }
                if is_pinned && !pin_ray.contains(to) {
                    continue;
                }
                if check_count == 1 {
                    let checker_sq = checkers.lsb().unwrap();
                    if cap_sq != checker_sq {
                        continue;
                    }
                }
                let mut occ_ep = occ;
                occ_ep.0 ^= Bitboard::from_sq(from).0 | Bitboard::from_sq(cap_sq).0;
                occ_ep.0 |= Bitboard::from_sq(to).0;
                if is_square_attacked_with_occ(pos, king_sq, them, occ_ep) {
                    continue;
                }
                moves.push(Move::new(from, to, None));
            } else {
                if !them_occ.contains(to) {
                    continue;
                }
                if is_pinned && !pin_ray.contains(to) {
                    continue;
                }
                if check_count == 1 && !check_ray.contains(to) {
                    continue;
                }
                if nr as u8 == promo_rank {
                    for promo in [
                        PieceType::Queen,
                        PieceType::Rook,
                        PieceType::Bishop,
                        PieceType::Knight,
                    ] {
                        moves.push(Move::new(from, to, Some(promo)));
                    }
                } else {
                    moves.push(Move::new(from, to, None));
                }
            }
        }
        // Promo pushes (quiet but treated as tactical)
        let nr = fr + dir;
        if (0..8).contains(&nr) && nr as u8 == promo_rank {
            let to = Square::from_coords(ff as u8, nr as u8);
            if !occ.contains(to) {
                if is_pinned && !pin_ray.contains(to) {
                    continue;
                }
                if check_count == 1 && !check_ray.contains(to) {
                    continue;
                }
                for promo in [
                    PieceType::Queen,
                    PieceType::Rook,
                    PieceType::Bishop,
                    PieceType::Knight,
                ] {
                    moves.push(Move::new(from, to, Some(promo)));
                }
            }
        }
    }

    // Knights (captures only)
    let knights = pos.pieces_bb(Piece::new(us, PieceType::Knight));
    for from in knights.squares() {
        if pinned.contains(from) {
            continue;
        }
        let mut at = knight_attacks(from) & them_occ;
        if check_count == 1 {
            at &= check_ray;
        }
        for to in at.squares() {
            moves.push(Move::new(from, to, None));
        }
    }
    // Bishops
    let bishops = pos.pieces_bb(Piece::new(us, PieceType::Bishop));
    for from in bishops.squares() {
        let mut at = bishop_attacks(from, occ) & them_occ;
        if pinned.contains(from) {
            at &= pin_rays[from.index() as usize];
        }
        if check_count == 1 {
            at &= check_ray;
        }
        for to in at.squares() {
            moves.push(Move::new(from, to, None));
        }
    }
    // Rooks
    let rooks = pos.pieces_bb(Piece::new(us, PieceType::Rook));
    for from in rooks.squares() {
        let mut at = rook_attacks(from, occ) & them_occ;
        if pinned.contains(from) {
            at &= pin_rays[from.index() as usize];
        }
        if check_count == 1 {
            at &= check_ray;
        }
        for to in at.squares() {
            moves.push(Move::new(from, to, None));
        }
    }
    // Queens
    let queens = pos.pieces_bb(Piece::new(us, PieceType::Queen));
    for from in queens.squares() {
        let mut at = queen_attacks(from, occ) & them_occ;
        if pinned.contains(from) {
            at &= pin_rays[from.index() as usize];
        }
        if check_count == 1 {
            at &= check_ray;
        }
        for to in at.squares() {
            moves.push(Move::new(from, to, None));
        }
    }
    // King captures already handled
}

/// Count legal moves (for perft bulk).
pub fn count_legal(pos: &Position) -> usize {
    let mut moves = Vec::new();
    generate_legal(pos, &mut moves);
    moves.len()
}

/// Generate legal moves into a stack MoveList (allocation-free hot path).
pub fn generate_legal_into(pos: &Position, list: &mut MoveList) {
    let us = pos.side_to_move();
    let them = us.opposite();
    let occ = pos.occupied();
    let us_occ = pos.occupied_color(us);
    let them_occ = pos.occupied_color(them);

    let king_sq = match pos.king_square(us) {
        Some(s) => s,
        None => return,
    };

    let checkers = get_checkers(pos, king_sq, them, occ);
    let check_count = checkers.count() as usize;
    let (pinned, pin_rays) = get_pinned(pos, king_sq, us, them, occ);

    {
        let king_bb = Bitboard::from_sq(king_sq);
        let occ_without_king = Bitboard(occ.0 ^ king_bb.0);
        let king_moves = king_attacks(king_sq) & !us_occ;
        for to in king_moves.squares() {
            if is_square_attacked_with_occ(pos, to, them, occ_without_king) {
                continue;
            }
            list.push(Move::new(king_sq, to, None));
        }
    }

    if check_count >= 2 {
        return;
    }

    let check_ray = if check_count == 1 {
        let checker_sq = checkers.lsb().unwrap();
        squares_between(king_sq, checker_sq) | checkers
    } else {
        Bitboard::ALL
    };

    let pawns = pos.pieces_bb(Piece::new(us, PieceType::Pawn));
    let dir: i8 = if us == Color::White { 1 } else { -1 };
    let start_rank: i8 = if us == Color::White { 1 } else { 6 };
    let promo_rank: u8 = if us == Color::White { 7 } else { 0 };
    let ep_sq = pos.en_passant();

    for from in pawns.squares() {
        let is_pinned = pinned.contains(from);
        let pin_ray = pin_rays[from.index() as usize];
        let fr = from.rank() as i8;
        let ff = from.file() as i8;

        let nr = fr + dir;
        if (0..8).contains(&nr) {
            let to = Square::from_coords(ff as u8, nr as u8);
            if !occ.contains(to) {
                let pinned_ok = !is_pinned || pin_ray.contains(to);
                let check_ok = check_count != 1 || check_ray.contains(to);
                if pinned_ok && check_ok {
                    if nr as u8 == promo_rank {
                        for promo in [
                            PieceType::Queen,
                            PieceType::Rook,
                            PieceType::Bishop,
                            PieceType::Knight,
                        ] {
                            list.push(Move::new(from, to, Some(promo)));
                        }
                    } else {
                        list.push(Move::new(from, to, None));
                    }
                }
                if fr == start_rank {
                    let nr2 = nr + dir;
                    if (0..8).contains(&nr2) {
                        let to2 = Square::from_coords(ff as u8, nr2 as u8);
                        if !occ.contains(to2) {
                            let pinned_ok2 = !is_pinned || pin_ray.contains(to2);
                            let check_ok2 = check_count != 1 || check_ray.contains(to2);
                            if pinned_ok2 && check_ok2 {
                                list.push(Move::new(from, to2, None));
                            }
                        }
                    }
                }
            }
        }

        for df in [-1_i8, 1] {
            let nf = ff + df;
            let nr2 = fr + dir;
            if !(0..8).contains(&nf) || !(0..8).contains(&nr2) {
                continue;
            }
            let to = Square::from_coords(nf as u8, nr2 as u8);
            let is_ep = ep_sq == Some(to);
            if is_ep {
                let cap_sq = Square::from_coords(nf as u8, fr as u8);
                let cap_piece = pos.piece_at(cap_sq);
                if cap_piece != Some(Piece::new(them, PieceType::Pawn)) {
                    continue;
                }
                if is_pinned && !pin_ray.contains(to) {
                    continue;
                }
                if check_count == 1 {
                    let checker_sq = checkers.lsb().unwrap();
                    if cap_sq != checker_sq {
                        continue;
                    }
                }
                let mut occ_ep = occ;
                occ_ep.0 ^= Bitboard::from_sq(from).0;
                occ_ep.0 ^= Bitboard::from_sq(cap_sq).0;
                occ_ep.0 |= Bitboard::from_sq(to).0;
                if is_square_attacked_with_occ(pos, king_sq, them, occ_ep) {
                    continue;
                }
                list.push(Move::new(from, to, None));
            } else {
                if !them_occ.contains(to) {
                    continue;
                }
                if is_pinned && !pin_ray.contains(to) {
                    continue;
                }
                if check_count == 1 && !check_ray.contains(to) {
                    continue;
                }
                if nr2 as u8 == promo_rank {
                    for promo in [
                        PieceType::Queen,
                        PieceType::Rook,
                        PieceType::Bishop,
                        PieceType::Knight,
                    ] {
                        list.push(Move::new(from, to, Some(promo)));
                    }
                } else {
                    list.push(Move::new(from, to, None));
                }
            }
        }
    }

    let knights = pos.pieces_bb(Piece::new(us, PieceType::Knight));
    for from in knights.squares() {
        if pinned.contains(from) {
            continue;
        }
        let mut attacks = knight_attacks(from) & !us_occ;
        if check_count == 1 {
            attacks &= check_ray;
        }
        for to in attacks.squares() {
            list.push(Move::new(from, to, None));
        }
    }

    let bishops = pos.pieces_bb(Piece::new(us, PieceType::Bishop));
    for from in bishops.squares() {
        let mut attacks = bishop_attacks(from, occ) & !us_occ;
        if pinned.contains(from) {
            attacks &= pin_rays[from.index() as usize];
        }
        if check_count == 1 {
            attacks &= check_ray;
        }
        for to in attacks.squares() {
            list.push(Move::new(from, to, None));
        }
    }

    let rooks = pos.pieces_bb(Piece::new(us, PieceType::Rook));
    for from in rooks.squares() {
        let mut attacks = rook_attacks(from, occ) & !us_occ;
        if pinned.contains(from) {
            attacks &= pin_rays[from.index() as usize];
        }
        if check_count == 1 {
            attacks &= check_ray;
        }
        for to in attacks.squares() {
            list.push(Move::new(from, to, None));
        }
    }

    let queens = pos.pieces_bb(Piece::new(us, PieceType::Queen));
    for from in queens.squares() {
        let mut attacks = queen_attacks(from, occ) & !us_occ;
        if pinned.contains(from) {
            attacks &= pin_rays[from.index() as usize];
        }
        if check_count == 1 {
            attacks &= check_ray;
        }
        for to in attacks.squares() {
            list.push(Move::new(from, to, None));
        }
    }

    if check_count != 0 {
        return;
    }
    if us == Color::White {
        if pos.castling() & CASTLE_WK != 0 {
            if pos.piece_at(F1).is_none()
                && pos.piece_at(G1).is_none()
                && pos.piece_at(H1_SQ) == Some(Piece::WR)
                && !is_square_attacked(pos, E1, them)
                && !is_square_attacked(pos, F1, them)
                && !is_square_attacked(pos, G1, them)
            {
                list.push(Move::new(E1, G1, None));
            }
        }
        if pos.castling() & CASTLE_WQ != 0 {
            if pos.piece_at(D1).is_none()
                && pos.piece_at(C1).is_none()
                && pos.piece_at(B1).is_none()
                && pos.piece_at(A1_SQ) == Some(Piece::WR)
                && !is_square_attacked(pos, E1, them)
                && !is_square_attacked(pos, D1, them)
                && !is_square_attacked(pos, C1, them)
            {
                list.push(Move::new(E1, C1, None));
            }
        }
    } else {
        if pos.castling() & CASTLE_BK != 0 {
            if pos.piece_at(F8).is_none()
                && pos.piece_at(G8).is_none()
                && pos.piece_at(H8_SQ) == Some(Piece::BR)
                && !is_square_attacked(pos, E8, them)
                && !is_square_attacked(pos, F8, them)
                && !is_square_attacked(pos, G8, them)
            {
                list.push(Move::new(E8, G8, None));
            }
        }
        if pos.castling() & CASTLE_BQ != 0 {
            if pos.piece_at(D8).is_none()
                && pos.piece_at(C8).is_none()
                && pos.piece_at(B8).is_none()
                && pos.piece_at(A8_SQ) == Some(Piece::BR)
                && !is_square_attacked(pos, E8, them)
                && !is_square_attacked(pos, D8, them)
                && !is_square_attacked(pos, C8, them)
            {
                list.push(Move::new(E8, C8, None));
            }
        }
    }
}

/// Generate legal captures into a stack MoveList.
pub fn generate_captures_into(pos: &Position, list: &mut MoveList) {
    let us = pos.side_to_move();
    let them = us.opposite();
    let occ = pos.occupied();
    let them_occ = pos.occupied_color(them);

    let king_sq = match pos.king_square(us) {
        Some(s) => s,
        None => return,
    };

    let checkers = get_checkers(pos, king_sq, them, occ);
    let check_count = checkers.count() as usize;
    let (pinned, pin_rays) = get_pinned(pos, king_sq, us, them, occ);

    {
        let king_bb = Bitboard::from_sq(king_sq);
        let occ_without_king = Bitboard(occ.0 ^ king_bb.0);
        let king_caps = king_attacks(king_sq) & them_occ;
        for to in king_caps.squares() {
            if is_square_attacked_with_occ(pos, to, them, occ_without_king) {
                continue;
            }
            list.push(Move::new(king_sq, to, None));
        }
        if check_count >= 2 {
            return;
        }
    }

    let check_ray = if check_count == 1 {
        let checker_sq = checkers.lsb().unwrap();
        squares_between(king_sq, checker_sq) | checkers
    } else {
        Bitboard::ALL
    };

    let pawns = pos.pieces_bb(Piece::new(us, PieceType::Pawn));
    let dir: i8 = if us == Color::White { 1 } else { -1 };
    let promo_rank: u8 = if us == Color::White { 7 } else { 0 };
    let ep_sq = pos.en_passant();

    for from in pawns.squares() {
        let is_pinned = pinned.contains(from);
        let pin_ray = pin_rays[from.index() as usize];
        let fr = from.rank() as i8;
        let ff = from.file() as i8;
        for df in [-1_i8, 1] {
            let nf = ff + df;
            let nr = fr + dir;
            if !(0..8).contains(&nf) || !(0..8).contains(&nr) {
                continue;
            }
            let to = Square::from_coords(nf as u8, nr as u8);
            let is_ep = ep_sq == Some(to);
            if is_ep {
                let cap_sq = Square::from_coords(nf as u8, fr as u8);
                if pos.piece_at(cap_sq) != Some(Piece::new(them, PieceType::Pawn)) {
                    continue;
                }
                if is_pinned && !pin_ray.contains(to) {
                    continue;
                }
                if check_count == 1 {
                    let checker_sq = checkers.lsb().unwrap();
                    if cap_sq != checker_sq {
                        continue;
                    }
                }
                let mut occ_ep = occ;
                occ_ep.0 ^= Bitboard::from_sq(from).0 | Bitboard::from_sq(cap_sq).0;
                occ_ep.0 |= Bitboard::from_sq(to).0;
                if is_square_attacked_with_occ(pos, king_sq, them, occ_ep) {
                    continue;
                }
                list.push(Move::new(from, to, None));
            } else {
                if !them_occ.contains(to) {
                    continue;
                }
                if is_pinned && !pin_ray.contains(to) {
                    continue;
                }
                if check_count == 1 && !check_ray.contains(to) {
                    continue;
                }
                if nr as u8 == promo_rank {
                    for promo in [
                        PieceType::Queen,
                        PieceType::Rook,
                        PieceType::Bishop,
                        PieceType::Knight,
                    ] {
                        list.push(Move::new(from, to, Some(promo)));
                    }
                } else {
                    list.push(Move::new(from, to, None));
                }
            }
        }
        let nr = fr + dir;
        if (0..8).contains(&nr) && nr as u8 == promo_rank {
            let to = Square::from_coords(ff as u8, nr as u8);
            if !occ.contains(to) {
                if is_pinned && !pin_ray.contains(to) {
                    continue;
                }
                if check_count == 1 && !check_ray.contains(to) {
                    continue;
                }
                for promo in [
                    PieceType::Queen,
                    PieceType::Rook,
                    PieceType::Bishop,
                    PieceType::Knight,
                ] {
                    list.push(Move::new(from, to, Some(promo)));
                }
            }
        }
    }

    let knights = pos.pieces_bb(Piece::new(us, PieceType::Knight));
    for from in knights.squares() {
        if pinned.contains(from) {
            continue;
        }
        let mut at = knight_attacks(from) & them_occ;
        if check_count == 1 {
            at &= check_ray;
        }
        for to in at.squares() {
            list.push(Move::new(from, to, None));
        }
    }
    let bishops = pos.pieces_bb(Piece::new(us, PieceType::Bishop));
    for from in bishops.squares() {
        let mut at = bishop_attacks(from, occ) & them_occ;
        if pinned.contains(from) {
            at &= pin_rays[from.index() as usize];
        }
        if check_count == 1 {
            at &= check_ray;
        }
        for to in at.squares() {
            list.push(Move::new(from, to, None));
        }
    }
    let rooks = pos.pieces_bb(Piece::new(us, PieceType::Rook));
    for from in rooks.squares() {
        let mut at = rook_attacks(from, occ) & them_occ;
        if pinned.contains(from) {
            at &= pin_rays[from.index() as usize];
        }
        if check_count == 1 {
            at &= check_ray;
        }
        for to in at.squares() {
            list.push(Move::new(from, to, None));
        }
    }
    let queens = pos.pieces_bb(Piece::new(us, PieceType::Queen));
    for from in queens.squares() {
        let mut at = queen_attacks(from, occ) & them_occ;
        if pinned.contains(from) {
            at &= pin_rays[from.index() as usize];
        }
        if check_count == 1 {
            at &= check_ray;
        }
        for to in at.squares() {
            list.push(Move::new(from, to, None));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::parse_fen;

    #[test]
    fn startpos_legal_count() {
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let mut moves = Vec::new();
        generate_legal(&pos, &mut moves);
        assert_eq!(moves.len(), 20);
    }

    #[test]
    fn kiwipete_legal_count() {
        let pos = parse_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
            .unwrap();
        let mut moves = Vec::new();
        generate_legal(&pos, &mut moves);
        assert_eq!(moves.len(), 48);
    }

    #[test]
    fn is_attacked_basic() {
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let e1 = Square::from_str("e1").unwrap();
        assert!(!is_square_attacked(&pos, e1, Color::Black));
        let d3 = Square::from_str("d3").unwrap();
        assert!(is_square_attacked(&pos, d3, Color::White));
    }
}

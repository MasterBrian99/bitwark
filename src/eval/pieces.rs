//! Piece terms — mobility, rook open files, bishop pair.
//!
//! Mobility counts pseudo-legal attack squares not occupied by own pieces and
//! not defended by enemy pawns (chessprogramming.org "Mobility"). Rook on an
//! open/semi-open file and the bishop pair are classical bonuses
//! (chessprogramming.org "Rook on Open File", "Bishop Pair").

use crate::board::{
    Bitboard, Color, Piece, PieceType, Position, Square,
    attacks::{bishop_attacks, knight_attacks, pawn_attacks, queen_attacks, rook_attacks},
};

// Weights per mobility square (MG, EG). Small so PeSTO PSTs aren't double-counted.
const MOBILITY_N_MG: i32 = 4;
const MOBILITY_N_EG: i32 = 4;
const MOBILITY_B_MG: i32 = 4;
const MOBILITY_B_EG: i32 = 4;
const MOBILITY_R_MG: i32 = 2;
const MOBILITY_R_EG: i32 = 2;
const MOBILITY_Q_MG: i32 = 1;
const MOBILITY_Q_EG: i32 = 1;

// Rook file bonuses (white − black). Symmetric.
const ROOK_OPEN_MG: i32 = 25;
const ROOK_OPEN_EG: i32 = 12;
const ROOK_SEMI_MG: i32 = 12;
const ROOK_SEMI_EG: i32 = 6;

// Bishop pair bonus (side with >=2 bishops).
const BISHOP_PAIR_MG: i32 = 20;
const BISHOP_PAIR_EG: i32 = 40;

// Outposts (N+B in enemy half, not pawn-attackable, defended).
const OUTPOST_N_MG: i32 = 12;
const OUTPOST_N_EG: i32 = 20;
const OUTPOST_B_MG: i32 = 8;
const OUTPOST_B_EG: i32 = 12;

// Bad bishop (own pawns on bishop color).
const BAD_BISHOP_MG: i32 = -4;
const BAD_BISHOP_EG: i32 = -8;

// Trapped pieces.
const TRAPPED_ROOK_MG: i32 = -40;
const TRAPPED_ROOK_EG: i32 = -30;
const TRAPPED_KNIGHT_MG: i32 = -50;
const TRAPPED_KNIGHT_EG: i32 = -40;
const TRAPPED_BISHOP_MG: i32 = -30;
const TRAPPED_BISHOP_EG: i32 = -25;

// Rook on 7th.
const ROOK_7TH_MG: i32 = 15;
const ROOK_7TH_EG: i32 = 30;
const ROOK_7TH_KING_MG: i32 = 5;
const ROOK_7TH_KING_EG: i32 = 15;

/// Mobility (mg, eg) white − black.
pub fn mobility(pos: &Position) -> (i32, i32) {
    let occupied = pos.occupied();
    let white_occ = pos.occupied_color(Color::White);
    let black_occ = pos.occupied_color(Color::Black);
    let white_pawn_attacks = pawn_attacks_bb(pos, Color::White);
    let black_pawn_attacks = pawn_attacks_bb(pos, Color::Black);

    let mut mg: i32 = 0;
    let mut eg: i32 = 0;

    for &color in &[Color::White, Color::Black] {
        let (own_occ, enemy_pawn_attacks) = if color == Color::White {
            (white_occ, black_pawn_attacks)
        } else {
            (black_occ, white_pawn_attacks)
        };
        let sign = if color == Color::White { 1 } else { -1 };

        // Knights
        for sq in pos
            .pieces_bb(Piece::new(color, PieceType::Knight))
            .squares()
        {
            let attacks = knight_attacks(sq);
            let mob = (attacks.0 & !own_occ.0 & !enemy_pawn_attacks.0).count_ones() as i32;
            mg += sign * mob * MOBILITY_N_MG;
            eg += sign * mob * MOBILITY_N_EG;
        }
        // Bishops
        for sq in pos
            .pieces_bb(Piece::new(color, PieceType::Bishop))
            .squares()
        {
            let attacks = bishop_attacks(sq, occupied);
            let mob = (attacks.0 & !own_occ.0 & !enemy_pawn_attacks.0).count_ones() as i32;
            mg += sign * mob * MOBILITY_B_MG;
            eg += sign * mob * MOBILITY_B_EG;
        }
        // Rooks
        for sq in pos.pieces_bb(Piece::new(color, PieceType::Rook)).squares() {
            let attacks = rook_attacks(sq, occupied);
            let mob = (attacks.0 & !own_occ.0 & !enemy_pawn_attacks.0).count_ones() as i32;
            mg += sign * mob * MOBILITY_R_MG;
            eg += sign * mob * MOBILITY_R_EG;
        }
        // Queens
        for sq in pos.pieces_bb(Piece::new(color, PieceType::Queen)).squares() {
            let attacks = queen_attacks(sq, occupied);
            let mob = (attacks.0 & !own_occ.0 & !enemy_pawn_attacks.0).count_ones() as i32;
            mg += sign * mob * MOBILITY_Q_MG;
            eg += sign * mob * MOBILITY_Q_EG;
        }
    }

    (mg, eg)
}

/// Rook on open/semi-open file bonus, white − black.
pub fn rook_files(pos: &Position) -> (i32, i32) {
    let white_pawns = pos.pieces_bb(Piece::new(Color::White, PieceType::Pawn));
    let black_pawns = pos.pieces_bb(Piece::new(Color::Black, PieceType::Pawn));

    let mut mg: i32 = 0;
    let mut eg: i32 = 0;

    for &color in &[Color::White, Color::Black] {
        let (own_pawns, enemy_pawns) = if color == Color::White {
            (white_pawns, black_pawns)
        } else {
            (black_pawns, white_pawns)
        };
        let sign = if color == Color::White { 1 } else { -1 };
        for sq in pos.pieces_bb(Piece::new(color, PieceType::Rook)).squares() {
            let file = sq.file();
            let file_bb = Bitboard::file_bb(file);
            let has_own = (own_pawns.0 & file_bb.0) != 0;
            let has_enemy = (enemy_pawns.0 & file_bb.0) != 0;
            if !has_own && !has_enemy {
                mg += sign * ROOK_OPEN_MG;
                eg += sign * ROOK_OPEN_EG;
            } else if !has_own && has_enemy {
                mg += sign * ROOK_SEMI_MG;
                eg += sign * ROOK_SEMI_EG;
            }
        }
    }
    (mg, eg)
}

/// Bishop pair bonus, white − black. +bonus if side has >=2 bishops.
pub fn bishop_pair(pos: &Position) -> (i32, i32) {
    let white_bishops = pos
        .pieces_bb(Piece::new(Color::White, PieceType::Bishop))
        .count() as i32;
    let black_bishops = pos
        .pieces_bb(Piece::new(Color::Black, PieceType::Bishop))
        .count() as i32;
    let mut mg = 0;
    let mut eg = 0;
    if white_bishops >= 2 {
        mg += BISHOP_PAIR_MG;
        eg += BISHOP_PAIR_EG;
    }
    if black_bishops >= 2 {
        mg -= BISHOP_PAIR_MG;
        eg -= BISHOP_PAIR_EG;
    }
    (mg, eg)
}

/// Outposts (N/B) — white − black.
pub fn outposts(pos: &Position) -> (i32, i32) {
    let mut mg = 0;
    let mut eg = 0;
    for &color in &[Color::White, Color::Black] {
        let sign = if color == Color::White { 1 } else { -1 };
        let enemy = color.opposite();
        let own_pawns = pos.pieces_bb(Piece::new(color, PieceType::Pawn));
        let enemy_pawns = pos.pieces_bb(Piece::new(enemy, PieceType::Pawn));
        for sq in pos
            .pieces_bb(Piece::new(color, PieceType::Knight))
            .squares()
        {
            if is_outpost(sq, color, own_pawns, enemy_pawns) {
                mg += sign * OUTPOST_N_MG;
                eg += sign * OUTPOST_N_EG;
            }
        }
        for sq in pos
            .pieces_bb(Piece::new(color, PieceType::Bishop))
            .squares()
        {
            if is_outpost(sq, color, own_pawns, enemy_pawns) {
                mg += sign * OUTPOST_B_MG;
                eg += sign * OUTPOST_B_EG;
            }
        }
    }
    (mg, eg)
}

/// Bad bishop — penalty per own pawn on bishop color, white − black.
pub fn bad_bishop(pos: &Position) -> (i32, i32) {
    let mut mg = 0;
    let mut eg = 0;
    for &color in &[Color::White, Color::Black] {
        let sign = if color == Color::White { 1 } else { -1 };
        let bishops = pos.pieces_bb(Piece::new(color, PieceType::Bishop));
        if bishops.is_empty() {
            continue;
        }
        let own_pawns = pos.pieces_bb(Piece::new(color, PieceType::Pawn));
        // Count pawns on light/dark per bishop color? Simplify: for each bishop,
        // count pawns on its color and sum.
        for sq in bishops.squares() {
            let bishop_is_light = (sq.file() as i32 + sq.rank() as i32) % 2 == 1;
            let mut cnt = 0;
            for psq in own_pawns.squares() {
                let pawn_is_light = (psq.file() as i32 + psq.rank() as i32) % 2 == 1;
                if pawn_is_light == bishop_is_light {
                    cnt += 1;
                }
            }
            mg += sign * cnt * BAD_BISHOP_MG;
            eg += sign * cnt * BAD_BISHOP_EG;
        }
    }
    (mg, eg)
}

/// Trapped pieces (R/N/B corners), white − black.
pub fn trapped(pos: &Position) -> (i32, i32) {
    let mut mg = 0;
    let mut eg = 0;
    for &color in &[Color::White, Color::Black] {
        let sign = if color == Color::White { 1 } else { -1 };
        let enemy = color.opposite();
        let enemy_pawns = pos.pieces_bb(Piece::new(enemy, PieceType::Pawn));
        let own_pawns = pos.pieces_bb(Piece::new(color, PieceType::Pawn));
        for sq in pos.pieces_bb(Piece::new(color, PieceType::Rook)).squares() {
            if is_trapped_rook(sq, color, enemy_pawns, own_pawns) {
                mg += sign * TRAPPED_ROOK_MG;
                eg += sign * TRAPPED_ROOK_EG;
            }
        }
        for sq in pos
            .pieces_bb(Piece::new(color, PieceType::Knight))
            .squares()
        {
            if is_trapped_knight(sq, color, enemy_pawns) {
                mg += sign * TRAPPED_KNIGHT_MG;
                eg += sign * TRAPPED_KNIGHT_EG;
            }
        }
        for sq in pos
            .pieces_bb(Piece::new(color, PieceType::Bishop))
            .squares()
        {
            if is_trapped_bishop(sq, color, own_pawns) {
                mg += sign * TRAPPED_BISHOP_MG;
                eg += sign * TRAPPED_BISHOP_EG;
            }
        }
    }
    (mg, eg)
}

/// Rook on 7th rank (relative to enemy), white − black.
pub fn rook_seventh(pos: &Position) -> (i32, i32) {
    let mut mg = 0;
    let mut eg = 0;
    let w_king = pos.king_square(Color::Black);
    let b_king = pos.king_square(Color::White);
    for sq in pos
        .pieces_bb(Piece::new(Color::White, PieceType::Rook))
        .squares()
    {
        if sq.rank() == 6 {
            mg += ROOK_7TH_MG;
            eg += ROOK_7TH_EG;
            if let Some(ek) = w_king {
                if ek.rank() == 7 {
                    mg += ROOK_7TH_KING_MG;
                    eg += ROOK_7TH_KING_EG;
                }
            }
        }
    }
    for sq in pos
        .pieces_bb(Piece::new(Color::Black, PieceType::Rook))
        .squares()
    {
        if sq.rank() == 1 {
            mg -= ROOK_7TH_MG;
            eg -= ROOK_7TH_EG;
            if let Some(ek) = b_king {
                if ek.rank() == 0 {
                    mg -= ROOK_7TH_KING_MG;
                    eg -= ROOK_7TH_KING_EG;
                }
            }
        }
    }
    (mg, eg)
}

fn is_outpost(sq: Square, color: Color, own_pawns: Bitboard, enemy_pawns: Bitboard) -> bool {
    let rank = sq.rank() as i32;
    let file = sq.file() as i32;
    // In enemy half?
    let in_enemy_half = if color == Color::White {
        rank >= 4
    } else {
        rank <= 3
    };
    if !in_enemy_half {
        return false;
    }
    // Not pawn-attackable: no enemy pawn on adjacent files ahead (>= rank for white, <= rank for black).
    let mut pawn_attackable = false;
    for df in [-1, 1] {
        let f = file + df;
        if !(0..8).contains(&f) {
            continue;
        }
        for r in 0..8 {
            let ahead = if color == Color::White {
                r > rank
            } else {
                r < rank
            };
            if !ahead {
                continue;
            }
            let s = Square::new((r * 8 + f) as u8);
            if enemy_pawns.contains(s) {
                pawn_attackable = true;
                break;
            }
        }
        if pawn_attackable {
            break;
        }
    }
    if pawn_attackable {
        return false;
    }
    // Defended by own pawn? Own pawn at (f±1, r-1) for White or (f±1, r+1) for Black attacks sq.
    let defender_rank = if color == Color::White {
        rank - 1
    } else {
        rank + 1
    };
    if !(0..8).contains(&defender_rank) {
        return false;
    }
    for df in [-1, 1] {
        let f = file + df;
        if !(0..8).contains(&f) {
            continue;
        }
        let s = Square::new((defender_rank * 8 + f) as u8);
        if own_pawns.contains(s) {
            return true;
        }
    }
    false
}

fn is_trapped_rook(sq: Square, color: Color, enemy_pawns: Bitboard, _own_pawns: Bitboard) -> bool {
    let file = sq.file() as i32;
    let rank = sq.rank() as i32;
    if color == Color::White {
        if (rank == 7 || rank == 6) && (file == 0 || file == 7) {
            let adj_file = if file == 0 { 1 } else { 6 };
            // Enemy pawn on b7/b6 or g7/g6?
            for r in [6, 5] {
                let s = Square::new((r * 8 + adj_file) as u8);
                if enemy_pawns.contains(s) {
                    return true;
                }
            }
        }
    } else {
        if (rank == 0 || rank == 1) && (file == 0 || file == 7) {
            let adj_file = if file == 0 { 1 } else { 6 };
            for r in [1, 2] {
                let s = Square::new((r * 8 + adj_file) as u8);
                if enemy_pawns.contains(s) {
                    return true;
                }
            }
        }
    }
    false
}

fn is_trapped_knight(sq: Square, color: Color, enemy_pawns: Bitboard) -> bool {
    let file = sq.file() as i32;
    let rank = sq.rank() as i32;
    if color == Color::White {
        // White knight trapped on a7 (0,6), a8 (0,7), h7 (7,6), h8 (7,7), b8 (1,7), g8 (6,7)
        let corners = [(0, 6), (0, 7), (7, 6), (7, 7), (1, 7), (6, 7)];
        if !corners.contains(&(file, rank)) {
            return false;
        }
        // Walled by enemy pawns on b6/b7 etc.
        for &(pf, pr) in &[(1, 5), (1, 6), (6, 5), (6, 6), (2, 6)] {
            let s = Square::new((pr * 8 + pf) as u8);
            if enemy_pawns.contains(s) {
                return true;
            }
        }
        false
    } else {
        let corners = [(0, 0), (0, 1), (7, 0), (7, 1), (1, 0), (6, 0)];
        if !corners.contains(&(file, rank)) {
            return false;
        }
        for &(pf, pr) in &[(1, 1), (1, 2), (6, 1), (6, 2), (2, 1)] {
            let s = Square::new((pr * 8 + pf) as u8);
            if enemy_pawns.contains(s) {
                return true;
            }
        }
        false
    }
}

fn is_trapped_bishop(sq: Square, color: Color, own_pawns: Bitboard) -> bool {
    let file = sq.file() as i32;
    let rank = sq.rank() as i32;
    if color == Color::White {
        let corners = [(0, 6), (0, 7), (7, 6), (7, 7)];
        if !corners.contains(&(file, rank)) {
            return false;
        }
        // Blocked by own pawns on adjacent diagonals
        let dirs = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
        for (df, dr) in dirs {
            let nf = file + df;
            let nr = rank + dr;
            if (0..8).contains(&nf) && (0..8).contains(&nr) {
                let s = Square::new((nr * 8 + nf) as u8);
                if own_pawns.contains(s) {
                    return true;
                }
            }
        }
        false
    } else {
        let corners = [(0, 0), (0, 1), (7, 0), (7, 1)];
        if !corners.contains(&(file, rank)) {
            return false;
        }
        let dirs = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
        for (df, dr) in dirs {
            let nf = file + df;
            let nr = rank + dr;
            if (0..8).contains(&nf) && (0..8).contains(&nr) {
                let s = Square::new((nr * 8 + nf) as u8);
                if own_pawns.contains(s) {
                    return true;
                }
            }
        }
        false
    }
}

fn pawn_attacks_bb(pos: &Position, color: Color) -> Bitboard {
    let mut bb = Bitboard::EMPTY;
    for sq in pos.pieces_bb(Piece::new(color, PieceType::Pawn)).squares() {
        bb = Bitboard(bb.0 | pawn_attacks(sq, color).0);
    }
    bb
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::parse_fen;

    #[test]
    fn rook_open_vs_closed() {
        // White rook on a1, file a with no pawns => open; vs file with pawns => closed
        let pos_open = parse_fen("4k3/8/8/8/8/8/8/R3K3 w - - 0 1").unwrap();
        let pos_closed = parse_fen("4k3/8/8/8/8/8/P7/R3K3 w - - 0 1").unwrap();
        let open = rook_files(&pos_open);
        let closed = rook_files(&pos_closed);
        assert!(open.0 > closed.0, "open {open:?} vs closed {closed:?}");
    }

    #[test]
    fn bishop_pair_bonus() {
        let pos_pair = parse_fen("4k3/8/8/8/8/8/8/BBK5 w - - 0 1").unwrap();
        let pos_single = parse_fen("4k3/8/8/8/8/8/8/B1K5 w - - 0 1").unwrap();
        let p = bishop_pair(&pos_pair);
        let s = bishop_pair(&pos_single);
        assert!(p.0 > s.0, "pair {p:?} vs single {s:?}");
        // Symmetry
        let pos_black_pair = parse_fen("4k3/8/8/8/8/8/8/4K2b w - - 0 1").unwrap();
        let pos_black_two = parse_fen("bb2k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let b = bishop_pair(&pos_black_two);
        assert!(b.0 < 0, "black pair should be negative {b:?}");
        let _ = pos_black_pair;
    }

    #[test]
    fn mobility_center_vs_corner() {
        // Bishop on d4 should have more mobility than on a1
        let pos_center = parse_fen("4k3/8/8/8/3B4/8/8/4K3 w - - 0 1").unwrap();
        let pos_corner = parse_fen("4k3/8/8/8/8/8/8/B3K3 w - - 0 1").unwrap();
        let c = mobility(&pos_center);
        let r = mobility(&pos_corner);
        assert!(c.0 > r.0, "center {c:?} vs corner {r:?}");
    }

    #[test]
    fn outpost_bonus() {
        // White knight d5 (3,4) with pawn c4 defending, no black pawn on c/e ahead → outpost
        let pos_outpost = parse_fen("4k3/8/8/3N4/2P5/8/8/4K3 w - - 0 1").unwrap(); // N d5, P c4
        let pos_no_outpost = parse_fen("4k3/3p4/8/3N4/8/8/8/4K3 w - - 0 1").unwrap(); // N d5, pawn c7 attacks?
        // Actually black pawn c7 (2,6) is on c file ahead of d5, blocks outpost
        let o = outposts(&pos_outpost);
        let n = outposts(&pos_no_outpost);
        assert!(o.0 > n.0, "outpost {o:?} vs non {n:?}");
    }

    #[test]
    fn bad_bishop_penalty() {
        // White bishop c1 with pawns on light squares vs off
        let pos_bad = parse_fen("4k3/8/8/8/8/8/PP6/B3K3 w - - 0 1").unwrap(); // pawns b2 c2 (light/dark?)
        // b2 (1,1) dark? file+rank 2 odd? Let's use bishop on c1 (light?) with pawns on same color
        // Simpler: bishop on c1 light, pawns b2(dark?) Actually c1 file2 rank0 => 2 even dark? Hmm.
        // Use pawns on same color: white pawns on light squares: b2(1,1)=0 even dark, c2(2,1)=1 light matches bishop a?
        // Just test that bad > good diff exists vs no bishop
        let pos_bad2 = parse_fen("4k3/8/8/8/8/8/3PP3/2B1K3 w - - 0 1").unwrap(); // pawns d2 e2, bishop c1
        let pos_good = parse_fen("4k3/8/8/8/8/8/8/2B1K3 w - - 0 1").unwrap();
        let b = bad_bishop(&pos_bad2);
        let g = bad_bishop(&pos_good);
        assert!(b.0 < g.0, "bad {b:?} vs good {g:?}");
    }

    #[test]
    fn trapped_rook() {
        // White rook a8 with black pawn b7 -> trapped
        let pos_trapped = parse_fen("R6k/1p6/8/8/8/8/8/4K3 w - - 0 1").unwrap(); // Ra8, pb7
        let pos_free = parse_fen("R6k/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let t = trapped(&pos_trapped);
        let f = trapped(&pos_free);
        assert!(t.0 < f.0, "trapped {t:?} vs free {f:?}");
    }

    #[test]
    fn rook_on_seventh() {
        let pos_7th = parse_fen("4k3/3R4/8/8/8/8/8/4K3 w - - 0 1").unwrap(); // R d7
        let pos_mid = parse_fen("4k3/8/8/3R4/8/8/8/4K3 w - - 0 1").unwrap(); // R d5
        let s7 = rook_seventh(&pos_7th);
        let sm = rook_seventh(&pos_mid);
        assert!(s7.0 > sm.0, "7th {s7:?} vs mid {sm:?}");
    }
}

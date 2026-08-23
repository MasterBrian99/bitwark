//! Evaluation — tapered handcrafted classical eval.
//!
//! The single-phase Michniewski eval
//! (chessprogramming.org "Simplified Evaluation Function") with a tapered
//! evaluation built on PeSTO's MG/EG tables
//! (chessprogramming.org "PeSTO's Evaluation Function" — Ronald Friederich,
//! RofChade, via Pawel Koziol's TSCP port). Two tables per piece (MG, EG)
//! are interpolated by game phase.
//!
//! Why tapering? A knight on the rim may be −50 cp in the middlegame but
//! only −20 in the endgame; a single table can't express both. Two tables
//! (MG, EG) interpolated by `phase = 0..24` do. See chessprogramming.org
//! "Tapered Eval".
//!
//! Scores are centipawns from the side-to-move's perspective (negamax
//! convention): positive = good for the player who must move. Tempo is a
//! small MG bonus for the side to move, scaled by phase. Evaluation is
//! computed from scratch each call (pawn structure is cached by hash).
//!
//! PSTs are stored with `a1 = 0` (`Square::index`). Black pieces mirror
//! vertically via `sq ^ 56` (flipping rank bits). Material is kept separate
//! from PST deltas so the `eval` breakdown can show both.
//!
//! Pawn structure, mobility/rook files/bishop pair, and king safety each contribute MG/EG deltas to the same
//! interpolated total.

use crate::board::{Bitboard, Color, Piece, PieceType, Position};

pub mod king;
pub mod pawns;
pub mod pieces;
pub mod tables;

/// Pawn structure cache — direct-mapped, keyed by pawn_hash.
/// Pure function cache: a miss recomputes, a collision overwrites.
/// Stores pawn-structure mg/eg plus passer bitboards (pawn-vs-pawn only,
/// hence cache-pure); passer scoring (king-dependent) lives outside the cache.
#[derive(Clone, Copy, Debug)]
struct PawnEntry {
    key: u64,
    mg: i32,
    eg: i32,
    w_passers: u64,
    b_passers: u64,
}

pub struct PawnCache {
    entries: Vec<PawnEntry>,
    mask: usize,
}

impl PawnCache {
    pub fn new() -> Self {
        let size = 1 << 14; // 16384 entries × ~32 bytes = 512 KiB
        Self {
            entries: vec![
                PawnEntry {
                    key: 0,
                    mg: 0,
                    eg: 0,
                    w_passers: 0,
                    b_passers: 0,
                };
                size
            ],
            mask: size - 1,
        }
    }

    #[inline]
    pub fn probe(&self, key: u64) -> Option<(i32, i32, Bitboard, Bitboard)> {
        let e = &self.entries[(key as usize) & self.mask];
        if e.key == key {
            Some((e.mg, e.eg, Bitboard(e.w_passers), Bitboard(e.b_passers)))
        } else {
            None
        }
    }

    #[inline]
    pub fn store(&mut self, key: u64, mg: i32, eg: i32, w: Bitboard, b: Bitboard) {
        let idx = (key as usize) & self.mask;
        self.entries[idx] = PawnEntry {
            key,
            mg,
            eg,
            w_passers: w.0,
            b_passers: b.0,
        };
    }
}

impl Default for PawnCache {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Term breakdown — the single source of truth for both search and `eval`
// ---------------------------------------------------------------------------

/// Tempo bonus (MG) for the side to move, tapered by phase.
///
/// 10 cp in the middlegame, ~0 in the endgame — the classical value used
/// alongside PeSTO.
pub const TEMPO_BONUS_MG: i32 = 10;

/// Number of scored terms.
pub const TERM_COUNT: usize = 14;
pub const TERM_MATERIAL: usize = 0;
pub const TERM_PST: usize = 1;
pub const TERM_PAWN: usize = 2;
pub const TERM_MOBILITY: usize = 3;
pub const TERM_ROOK: usize = 4;
pub const TERM_BISHOP_PAIR: usize = 5;
pub const TERM_KING_SHIELD: usize = 6;
pub const TERM_KING_ATTACK: usize = 7;
pub const TERM_PASSERS: usize = 8;
pub const TERM_KING_ACT: usize = 9;
pub const TERM_OUTPOSTS: usize = 10;
pub const TERM_BAD_BISHOP: usize = 11;
pub const TERM_TRAPPED: usize = 12;
pub const TERM_ROOK_7TH: usize = 13;
pub const TERM_NAMES: [&str; TERM_COUNT] = [
    "Material",
    "PieceSq",
    "PawnStruct",
    "Mobility",
    "RookFiles",
    "BishopPair",
    "KingShield",
    "KingAttack",
    "Passers",
    "KingAct",
    "Outposts",
    "BadBishop",
    "Trapped",
    "Rook7th",
];

/// Per-term MG/EG deltas, white − black. Built once per `breakdown()` call
/// and used both by `evaluate()` (search) and the `eval` debug command.
#[derive(Clone, Debug, Default)]
pub struct EvalBreakdown {
    /// Game phase 0..24 (24 = middlegame, 0 = endgame).
    pub phase: i32,
    /// Per-term MG deltas (white − black).
    pub term_mg: [i32; TERM_COUNT],
    /// Per-term EG deltas.
    pub term_eg: [i32; TERM_COUNT],
}

impl EvalBreakdown {
    #[inline]
    pub fn mg(&self) -> i32 {
        self.term_mg.iter().sum()
    }
    #[inline]
    pub fn eg(&self) -> i32 {
        self.term_eg.iter().sum()
    }
    /// White-perspective interpolated score (no tempo, no stm negation).
    #[inline]
    pub fn white_score(&self) -> i32 {
        let mg = self.mg();
        let eg = self.eg();
        (mg * self.phase + eg * (24 - self.phase)) / 24
    }
}

// ---------------------------------------------------------------------------
// Game phase
// ---------------------------------------------------------------------------

/// Compute game phase 0..24 from remaining piece material.
///
/// Knight/Bishop = 1, Rook = 2, Queen = 4 per side (PeSTO's
/// `gamephaseInc[]`). Pawn/King = 0.
pub fn game_phase(pos: &Position) -> i32 {
    let mut phase: i32 = 0;
    for &color in &[Color::White, Color::Black] {
        phase += pos.pieces_bb(Piece::new(color, PieceType::Knight)).count() as i32
            * tables::PHASE_WEIGHT[PieceType::Knight as usize];
        phase += pos.pieces_bb(Piece::new(color, PieceType::Bishop)).count() as i32
            * tables::PHASE_WEIGHT[PieceType::Bishop as usize];
        phase += pos.pieces_bb(Piece::new(color, PieceType::Rook)).count() as i32
            * tables::PHASE_WEIGHT[PieceType::Rook as usize];
        phase += pos.pieces_bb(Piece::new(color, PieceType::Queen)).count() as i32
            * tables::PHASE_WEIGHT[PieceType::Queen as usize];
    }
    phase.min(24)
}

// ---------------------------------------------------------------------------
// PST helpers — white perspective tables, black mirrors via sq ^ 56
// ---------------------------------------------------------------------------

#[inline]
fn pst_mg(pt: PieceType, sq_idx: usize) -> i32 {
    match pt {
        PieceType::Pawn => tables::MG_PAWN_TABLE[sq_idx],
        PieceType::Knight => tables::MG_KNIGHT_TABLE[sq_idx],
        PieceType::Bishop => tables::MG_BISHOP_TABLE[sq_idx],
        PieceType::Rook => tables::MG_ROOK_TABLE[sq_idx],
        PieceType::Queen => tables::MG_QUEEN_TABLE[sq_idx],
        PieceType::King => tables::MG_KING_TABLE[sq_idx],
    }
}

#[inline]
fn pst_eg(pt: PieceType, sq_idx: usize) -> i32 {
    match pt {
        PieceType::Pawn => tables::EG_PAWN_TABLE[sq_idx],
        PieceType::Knight => tables::EG_KNIGHT_TABLE[sq_idx],
        PieceType::Bishop => tables::EG_BISHOP_TABLE[sq_idx],
        PieceType::Rook => tables::EG_ROOK_TABLE[sq_idx],
        PieceType::Queen => tables::EG_QUEEN_TABLE[sq_idx],
        PieceType::King => tables::EG_KING_TABLE[sq_idx],
    }
}

// ---------------------------------------------------------------------------
// Core: breakdown and evaluate
// ---------------------------------------------------------------------------

/// Build the full MG/EG breakdown (white − black) for `pos`.
///
/// Iterates piece bitboards directly (no mailbox scan) — the right substrate
/// for pawn/file masks in later milestones. Missing kings (illegal FENs) are
/// tolerated: no panic.
pub fn breakdown(pos: &Position) -> EvalBreakdown {
    let phase = game_phase(pos);
    let mut bd = EvalBreakdown {
        phase,
        ..Default::default()
    };

    for &color in &[Color::White, Color::Black] {
        let sign: i32 = if color == Color::White { 1 } else { -1 };
        for &pt in &[
            PieceType::Pawn,
            PieceType::Knight,
            PieceType::Bishop,
            PieceType::Rook,
            PieceType::Queen,
            PieceType::King,
        ] {
            let bb = pos.pieces_bb(Piece::new(color, pt));
            if bb.is_empty() {
                continue;
            }
            let mat_mg = tables::MG_VALUE[pt as usize];
            let mat_eg = tables::EG_VALUE[pt as usize];
            // Count material once per piece occurrence (not per square loop for material?
            // Material is per piece, so multiply by popcount and handle PST per square.
            // We handle per-square loop for PST; accumulate material via popcount.
            let cnt = bb.count() as i32;
            bd.term_mg[TERM_MATERIAL] += sign * mat_mg * cnt;
            bd.term_eg[TERM_MATERIAL] += sign * mat_eg * cnt;

            for sq in bb.squares() {
                let idx = sq.index() as usize;
                let pst_idx = if color == Color::White { idx } else { idx ^ 56 };
                bd.term_mg[TERM_PST] += sign * pst_mg(pt, pst_idx);
                bd.term_eg[TERM_PST] += sign * pst_eg(pt, pst_idx);
            }
        }
    }

    // Pawn structure (5b) + 11b passers split
    let (pawn_mg, pawn_eg, w_pass, b_pass) = pawns::pawn_structure_and_passers(pos);
    bd.term_mg[TERM_PAWN] = pawn_mg;
    bd.term_eg[TERM_PAWN] = pawn_eg;
    let (pass_mg, pass_eg) = pawns::passer_score(pos, w_pass, b_pass);
    bd.term_mg[TERM_PASSERS] = pass_mg;
    bd.term_eg[TERM_PASSERS] = pass_eg;

    // Piece terms (5c)
    let (mob_mg, mob_eg) = pieces::mobility(pos);
    bd.term_mg[TERM_MOBILITY] = mob_mg;
    bd.term_eg[TERM_MOBILITY] = mob_eg;

    let (rook_mg, rook_eg) = pieces::rook_files(pos);
    bd.term_mg[TERM_ROOK] = rook_mg;
    bd.term_eg[TERM_ROOK] = rook_eg;

    let (bp_mg, bp_eg) = pieces::bishop_pair(pos);
    bd.term_mg[TERM_BISHOP_PAIR] = bp_mg;
    bd.term_eg[TERM_BISHOP_PAIR] = bp_eg;

    // King safety (5d -> 11a split)
    let (shield_mg, shield_eg) = king::shield(pos);
    bd.term_mg[TERM_KING_SHIELD] = shield_mg;
    bd.term_eg[TERM_KING_SHIELD] = shield_eg;
    let (attack_mg, attack_eg) = king::attack(pos);
    bd.term_mg[TERM_KING_ATTACK] = attack_mg;
    bd.term_eg[TERM_KING_ATTACK] = attack_eg;

    // King activity (11b EG)
    let (ka_mg, ka_eg) = king::king_activity(pos);
    bd.term_mg[TERM_KING_ACT] = ka_mg;
    bd.term_eg[TERM_KING_ACT] = ka_eg;

    // Piece depth (11d)
    let (out_mg, out_eg) = pieces::outposts(pos);
    bd.term_mg[TERM_OUTPOSTS] = out_mg;
    bd.term_eg[TERM_OUTPOSTS] = out_eg;
    let (bb_mg, bb_eg) = pieces::bad_bishop(pos);
    bd.term_mg[TERM_BAD_BISHOP] = bb_mg;
    bd.term_eg[TERM_BAD_BISHOP] = bb_eg;
    let (trap_mg, trap_eg) = pieces::trapped(pos);
    bd.term_mg[TERM_TRAPPED] = trap_mg;
    bd.term_eg[TERM_TRAPPED] = trap_eg;
    let (r7_mg, r7_eg) = pieces::rook_seventh(pos);
    bd.term_mg[TERM_ROOK_7TH] = r7_mg;
    bd.term_eg[TERM_ROOK_7TH] = r7_eg;

    bd
}

/// Evaluate `pos` in centipawns from the side-to-move's perspective.
///
/// Positive → good for the player to move (negamax convention). Uses the
/// tapered MG/EG total plus a small tempo bonus scaled by phase.
/// This is the incremental path: PSQT is read from `pos.psqt_*` (maintained
/// via `set_piece`), pawn structure is computed directly (no cache).
pub fn evaluate(pos: &Position) -> i32 {
    evaluate_with_pawn(pos, None)
}

/// Evaluate with pawn-hash cache (search hot path).
pub fn evaluate_cached(pos: &Position, cache: &mut PawnCache) -> i32 {
    evaluate_with_pawn(pos, Some(cache))
}

fn evaluate_with_pawn(pos: &Position, cache: Option<&mut PawnCache>) -> i32 {
    let phase = game_phase(pos);
    let psqt_mg = pos.psqt_mg();
    let psqt_eg = pos.psqt_eg();

    let (pawn_mg, pawn_eg, w_pass, b_pass) = if let Some(c) = cache {
        if let Some((mg, eg, w, b)) = c.probe(pos.pawn_hash()) {
            (mg, eg, w, b)
        } else {
            let (mg, eg, w, b) = pawns::pawn_structure_and_passers(pos);
            c.store(pos.pawn_hash(), mg, eg, w, b);
            (mg, eg, w, b)
        }
    } else {
        pawns::pawn_structure_and_passers(pos)
    };
    let (pass_mg, pass_eg) = pawns::passer_score(pos, w_pass, b_pass);

    let (mob_mg, mob_eg) = pieces::mobility(pos);
    let (rook_mg, rook_eg) = pieces::rook_files(pos);
    let (bp_mg, bp_eg) = pieces::bishop_pair(pos);
    let (shield_mg, shield_eg) = king::shield(pos);
    let (attack_mg, attack_eg) = king::attack(pos);
    let (ka_mg, ka_eg) = king::king_activity(pos);
    let (out_mg, out_eg) = pieces::outposts(pos);
    let (bb_mg, bb_eg) = pieces::bad_bishop(pos);
    let (trap_mg, trap_eg) = pieces::trapped(pos);
    let (r7_mg, r7_eg) = pieces::rook_seventh(pos);

    let mg = psqt_mg
        + pawn_mg
        + pass_mg
        + mob_mg
        + rook_mg
        + bp_mg
        + shield_mg
        + attack_mg
        + ka_mg
        + out_mg
        + bb_mg
        + trap_mg
        + r7_mg;
    let eg = psqt_eg
        + pawn_eg
        + pass_eg
        + mob_eg
        + rook_eg
        + bp_eg
        + shield_eg
        + attack_eg
        + ka_eg
        + out_eg
        + bb_eg
        + trap_eg
        + r7_eg;
    let white_score = (mg * phase + eg * (24 - phase)) / 24;

    let tempo_scaled = TEMPO_BONUS_MG * phase / 24;
    let white_pov = white_score
        + if pos.side_to_move() == Color::White {
            tempo_scaled
        } else {
            -tempo_scaled
        };
    if pos.side_to_move() == Color::White {
        white_pov
    } else {
        -white_pov
    }
}

/// Raw white-perspective interpolated score without tempo or stm negation.
///
/// Useful for symmetry tests (tempo would add a small side-to-move asymmetry).
#[allow(dead_code)]
pub fn evaluate_raw(pos: &Position) -> i32 {
    breakdown(pos).white_score()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fen::parse_fen;

    #[test]
    fn startpos_white_pov_zero() {
        let pos = crate::board::Position::startpos();
        // Symmetric PSTs + material → white-perspective 0; tempo not included in raw
        assert_eq!(evaluate_raw(&pos), 0);
        // evaluate() includes tempo for white to move → +10 scaled by phase 24 => 10
        // But we still check that startpos evaluate is small and symmetric around tempo
        let s = evaluate(&pos);
        assert!(
            s.abs() <= 15,
            "startpos stm score should be near tempo only, got {s}"
        );
    }

    #[test]
    fn startpos_black_to_move_symmetry() {
        let pos_w = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let pos_b = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1").unwrap();
        // White-perspective raw is 0 for both
        assert_eq!(evaluate_raw(&pos_w), 0);
        assert_eq!(evaluate_raw(&pos_b), 0);
        // stm scores should be equal (both get tempo for their side)
        assert_eq!(evaluate(&pos_w), evaluate(&pos_b));
    }

    #[test]
    fn mirror_symmetry_raw() {
        let fen_white = "4k3/8/8/8/4P3/8/8/4K3 w - - 0 1";
        let fen_black = "4k3/8/8/4p3/8/8/8/4K3 w - - 0 1";
        let pos_w = parse_fen(fen_white).unwrap();
        let pos_b = parse_fen(fen_black).unwrap();
        let raw_w = evaluate_raw(&pos_w);
        let raw_b = evaluate_raw(&pos_b);
        assert_eq!(raw_w, -raw_b);
    }

    #[test]
    fn pawn_mirror_pst() {
        let pos_white_e4 = parse_fen("4k3/8/8/8/4P3/8/8/4K3 w - - 0 1").unwrap();
        let pos_black_e5 = parse_fen("4k3/8/8/4p3/8/8/8/4K3 w - - 0 1").unwrap();
        let rw = evaluate_raw(&pos_white_e4);
        let rb = evaluate_raw(&pos_black_e5);
        assert_eq!(rw, -rb);
        assert!(rw != 0, "pawn should contribute non-zero with PeSTO tables");
    }

    #[test]
    fn queen_up_big_advantage() {
        let fen = "4k3/8/8/8/8/8/8/3QK3 w - - 0 1";
        let pos = parse_fen(fen).unwrap();
        let s = evaluate(&pos);
        assert!(s > 800, "queen up should be >800, got {s}");
        let fen_b = "4k3/8/8/8/8/8/8/3QK3 b - - 0 1";
        let pos_b = parse_fen(fen_b).unwrap();
        let s_b = evaluate(&pos_b);
        // Raw (white perspective) is side-independent → equal. Stm flips sign
        // but tempo creates a 2*tempo difference.
        assert_eq!(evaluate_raw(&pos), evaluate_raw(&pos_b));
        assert!(
            s_b < -800,
            "queen up but black to move should be < -800, got {s_b}"
        );
        assert!(
            (s + s_b).abs() <= 2 * TEMPO_BONUS_MG,
            "tempo diff too large: s={s} s_b={s_b}"
        );
    }

    #[test]
    fn knight_center_vs_rim() {
        let fen_center = "4k3/8/8/8/3N4/8/8/4K3 w - - 0 1";
        let fen_rim = "4k3/8/8/8/8/8/8/N3K3 w - - 0 1";
        let pos_c = parse_fen(fen_center).unwrap();
        let pos_r = parse_fen(fen_rim).unwrap();
        assert!(
            evaluate(&pos_c) > evaluate(&pos_r),
            "center {} vs rim {}",
            evaluate(&pos_c),
            evaluate(&pos_r)
        );
    }

    #[test]
    fn phase_extremes() {
        // K vs K: phase 0, PST still contributes but kings are symmetric → 0
        let fen_kk = "4k3/8/8/8/8/8/8/4K3 w - - 0 1";
        let pos = parse_fen(fen_kk).unwrap();
        assert_eq!(game_phase(&pos), 0);
        // Tempo scaled to 0 in endgame
        assert_eq!(evaluate(&pos), 0);
        // Startpos phase 24
        let pos2 = crate::board::Position::startpos();
        assert_eq!(game_phase(&pos2), 24);
    }

    #[test]
    fn tapered_mirror_color_flip() {
        // White knight on g1 vs black knight on g8 should be symmetric in raw
        let w = parse_fen("4k3/8/8/8/8/8/8/5NK1 w - - 0 1").unwrap(); // N on g1?
        let b = parse_fen("4k3/6n1/8/8/8/8/8/4K3 w - - 0 1").unwrap(); // n on g7? use g8 equivalent
        // Instead test a simpler color-flipped pair: white N on d4 vs black n on d5
        let w2 = parse_fen("4k3/8/8/8/3N4/8/8/4K3 w - - 0 1").unwrap();
        let b2 = parse_fen("4k3/8/8/3n4/8/8/8/4K3 w - - 0 1").unwrap();
        assert_eq!(evaluate_raw(&w2), -evaluate_raw(&b2));
    }

    #[test]
    fn no_panic_on_kingless() {
        let pos = crate::board::Position::empty_for_fen_internal();
        let _ = evaluate(&pos);
        let fen_kings = "4k3/8/8/8/8/8/8/4K3 w - - 0 1";
        let pos3 = parse_fen(fen_kings).unwrap();
        let _ = evaluate(&pos3);
    }

    #[test]
    fn material_phase_counts() {
        // 2 knights + 2 bishops + 4 rooks + 2 queens = 24
        let pos = parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        assert_eq!(game_phase(&pos), 24);
        // KQ vs K: one queen = phase 4
        let pos2 = parse_fen("4k3/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
        assert_eq!(game_phase(&pos2), 4);
        // Promotion overflow clamped at 24: 3 white queens + both kings
        let pos3 = parse_fen("QQQ5/8/8/8/8/8/8/4K2k w - - 0 1").unwrap();
        assert!(game_phase(&pos3) <= 24);
        // 6 white queens (phase 24, clamped)
        let pos5 = parse_fen("QQQ5/QQQ5/8/8/8/8/8/4K2k w - - 0 1").unwrap();
        assert_eq!(game_phase(&pos5), 24); // 6 queens = 24, clamped
    }

    #[test]
    fn evaluate_matches_breakdown() {
        let fens = [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "r1bq1rk1/pp2ppbp/2np1np1/2p5/2P1P3/1PN1B3/PB1Q1PPP/R3K2R w KQ - 0 1",
            "4k3/8/8/8/3P4/8/8/4K3 w - - 0 1",
            "4k3/8/8/4q3/2b5/8/8/4K3 w - - 0 1",
            "4k3/8/8/8/8/8/5PPP/5RK1 w - - 0 1",
            "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1",
            "4k3/3P4/8/8/8/8/8/4K3 w - - 0 1",
        ];
        for fen in fens {
            let pos = parse_fen(fen).unwrap();
            let bd = breakdown(&pos);
            let white_score = bd.white_score();
            let tempo_scaled = TEMPO_BONUS_MG * bd.phase / 24;
            let white_pov = white_score
                + if pos.side_to_move() == Color::White {
                    tempo_scaled
                } else {
                    -tempo_scaled
                };
            let stm_via_bd = if pos.side_to_move() == Color::White {
                white_pov
            } else {
                -white_pov
            };
            let ev = evaluate(&pos);
            assert_eq!(
                ev, stm_via_bd,
                "evaluate vs breakdown mismatch for {fen}: ev={ev} bd_stm={stm_via_bd} bd={bd:?}"
            );
            // Cached path must match too.
            let mut cache = PawnCache::new();
            let ev_cached = evaluate_cached(&pos, &mut cache);
            assert_eq!(
                ev_cached, ev,
                "cached vs uncached mismatch for {fen}: cached={ev_cached} ev={ev}"
            );
        }
    }
}

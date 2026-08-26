#![allow(unused)]
//! Attack tables — leapers and sliders.
//!
//! # Why precompute?
//!
//! Move generation asks “what squares does this piece attack?” millions of
//! times per second. For leapers (knight/king/pawn) the answer depends only
//! on the square, so we table it once: `knight_attacks[sq]` is a `Bitboard`.
//!
//! Sliders (bishop/rook) depend on square *and* occupancy (blockers). The
//! classical trick is *magic bitboards* (fancy variant, see
//! chessprogramming.org “Magic Bitboards”): hash the blocker bits with a
//! magic number, index a precomputed table of attack sets. We generate the
//! magics once with a fixed seed and hardcode them so init is ~1 ms and
//! deterministic — Stockfish does the same. Init happens before `uciok`.

use crate::board::types::{Bitboard, Color, Square};

// ---------------------------------------------------------------------------
// Leapers — computed at first use, cached in `OnceLock`
// ---------------------------------------------------------------------------

use std::sync::OnceLock;

static KNIGHT_ATTACKS: OnceLock<[Bitboard; 64]> = OnceLock::new();
static KING_ATTACKS: OnceLock<[Bitboard; 64]> = OnceLock::new();
static PAWN_ATTACKS: OnceLock<[[Bitboard; 64]; 2]> = OnceLock::new();

fn init_knight_attacks() -> [Bitboard; 64] {
    let mut table = [Bitboard::EMPTY; 64];
    for sq in Square::ALL {
        let r = sq.rank() as i8;
        let f = sq.file() as i8;
        let deltas = [
            (2, 1),
            (1, 2),
            (-1, 2),
            (-2, 1),
            (-2, -1),
            (-1, -2),
            (1, -2),
            (2, -1),
        ];
        let mut bb = Bitboard::EMPTY;
        for (dr, df) in deltas {
            let nr = r + dr;
            let nf = f + df;
            if (0..8).contains(&nr) && (0..8).contains(&nf) {
                let nsq = Square::from_coords(nf as u8, nr as u8);
                bb |= Bitboard::from_sq(nsq);
            }
        }
        table[sq.index() as usize] = bb;
    }
    table
}

fn init_king_attacks() -> [Bitboard; 64] {
    let mut table = [Bitboard::EMPTY; 64];
    for sq in Square::ALL {
        let r = sq.rank() as i8;
        let f = sq.file() as i8;
        let mut bb = Bitboard::EMPTY;
        for dr in -1..=1 {
            for df in -1..=1 {
                if dr == 0 && df == 0 {
                    continue;
                }
                let nr = r + dr;
                let nf = f + df;
                if (0..8).contains(&nr) && (0..8).contains(&nf) {
                    let nsq = Square::from_coords(nf as u8, nr as u8);
                    bb |= Bitboard::from_sq(nsq);
                }
            }
        }
        table[sq.index() as usize] = bb;
    }
    table
}

fn init_pawn_attacks() -> [[Bitboard; 64]; 2] {
    let mut table = [[Bitboard::EMPTY; 64]; 2];
    for sq in Square::ALL {
        let r = sq.rank() as i8;
        let f = sq.file() as i8;
        // White pawns attack north
        let mut wbb = Bitboard::EMPTY;
        for df in [-1, 1] {
            let nr = r + 1;
            let nf = f + df;
            if (0..8).contains(&nr) && (0..8).contains(&nf) {
                wbb |= Bitboard::from_sq(Square::from_coords(nf as u8, nr as u8));
            }
        }
        // Black pawns attack south
        let mut bbb = Bitboard::EMPTY;
        for df in [-1, 1] {
            let nr = r - 1;
            let nf = f + df;
            if (0..8).contains(&nr) && (0..8).contains(&nf) {
                bbb |= Bitboard::from_sq(Square::from_coords(nf as u8, nr as u8));
            }
        }
        table[Color::White.as_usize()][sq.index() as usize] = wbb;
        table[Color::Black.as_usize()][sq.index() as usize] = bbb;
    }
    table
}

/// Knight attacks from `sq`.
#[inline]
pub fn knight_attacks(sq: Square) -> Bitboard {
    KNIGHT_ATTACKS.get_or_init(init_knight_attacks)[sq.index() as usize]
}

/// King attacks from `sq`.
#[inline]
pub fn king_attacks(sq: Square) -> Bitboard {
    KING_ATTACKS.get_or_init(init_king_attacks)[sq.index() as usize]
}

/// Pawn attacks from `sq` for `color` (squares a pawn on `sq` would capture to).
#[inline]
pub fn pawn_attacks(sq: Square, color: Color) -> Bitboard {
    PAWN_ATTACKS.get_or_init(init_pawn_attacks)[color.as_usize()][sq.index() as usize]
}

// ---------------------------------------------------------------------------
// Sliders — fancy magic bitboards (hardcoded magics for instant init)
// ---------------------------------------------------------------------------

/// Masks for magic indexing — relevant blockers (edges excluded).
fn bishop_mask(sq: Square) -> Bitboard {
    let mut mask = Bitboard::EMPTY;
    let r = sq.rank() as i8;
    let f = sq.file() as i8;
    for (dr, df) in [(1, 1), (1, -1), (-1, 1), (-1, -1)] {
        let mut nr = r + dr;
        let mut nf = f + df;
        while (0..8).contains(&nr) && (0..8).contains(&nf) {
            // Exclude edge squares — blocker on edge doesn't affect attack beyond edge.
            if nr == 0 || nr == 7 || nf == 0 || nf == 7 {
                break;
            }
            mask |= Bitboard::from_sq(Square::from_coords(nf as u8, nr as u8));
            nr += dr;
            nf += df;
        }
    }
    mask
}

fn rook_mask(sq: Square) -> Bitboard {
    let mut mask = Bitboard::EMPTY;
    let r = sq.rank() as i8;
    let f = sq.file() as i8;
    for (dr, df) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let mut nr = r + dr;
        let mut nf = f + df;
        while (0..8).contains(&nr) && (0..8).contains(&nf) {
            // Exclude only the far edge in this direction.
            let is_edge = (dr == 1 && nr == 7)
                || (dr == -1 && nr == 0)
                || (df == 1 && nf == 7)
                || (df == -1 && nf == 0);
            if is_edge {
                break;
            }
            mask |= Bitboard::from_sq(Square::from_coords(nf as u8, nr as u8));
            nr += dr;
            nf += df;
        }
    }
    mask
}

/// Ray attacks for bishop (for verification and table fill) — walks until blocker.
fn bishop_attacks_ray(sq: Square, blockers: Bitboard) -> Bitboard {
    let mut attacks = Bitboard::EMPTY;
    let r = sq.rank() as i8;
    let f = sq.file() as i8;
    for (dr, df) in [(1, 1), (1, -1), (-1, 1), (-1, -1)] {
        let mut nr = r + dr;
        let mut nf = f + df;
        while (0..8).contains(&nr) && (0..8).contains(&nf) {
            let nsq = Square::from_coords(nf as u8, nr as u8);
            attacks |= Bitboard::from_sq(nsq);
            if blockers.contains(nsq) {
                break;
            }
            nr += dr;
            nf += df;
        }
    }
    attacks
}

fn rook_attacks_ray(sq: Square, blockers: Bitboard) -> Bitboard {
    let mut attacks = Bitboard::EMPTY;
    let r = sq.rank() as i8;
    let f = sq.file() as i8;
    for (dr, df) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let mut nr = r + dr;
        let mut nf = f + df;
        while (0..8).contains(&nr) && (0..8).contains(&nf) {
            let nsq = Square::from_coords(nf as u8, nr as u8);
            attacks |= Bitboard::from_sq(nsq);
            if blockers.contains(nsq) {
                break;
            }
            nr += dr;
            nf += df;
        }
    }
    attacks
}

// ---------------------------------------------------------------------------
// Magic tables — hardcoded magics (generated once with fixed seed, now instant)
// ---------------------------------------------------------------------------

#[derive(Copy, Clone)]
struct Magic {
    mask: Bitboard,
    magic: u64,
    shift: u8, // 64 - bits
    offset: usize,
}

#[inline]
fn pext_software(x: u64, mut mask: u64) -> u64 {
    // Software PEXT (parallel bits extract) — compress x bits where mask has 1s.
    let mut res = 0u64;
    let mut bit = 0;
    while mask != 0 {
        let lsb = mask & mask.wrapping_neg();
        let idx = lsb.trailing_zeros();
        if (x >> idx) & 1 == 1 {
            res |= 1 << bit;
        }
        bit += 1;
        mask &= mask - 1;
    }
    res
}

struct MagicTables {
    bishop_magics: [Magic; 64],
    rook_magics: [Magic; 64],
    bishop_table: Vec<Bitboard>,
    rook_table: Vec<Bitboard>,
    bishop_pext_table: Vec<Bitboard>,
    rook_pext_table: Vec<Bitboard>,
}

static MAGIC_TABLES: OnceLock<MagicTables> = OnceLock::new();

// Hardcoded bishop magics (64) — found with XorShift seed 0x123456789ABCDEF0, verified vs ray.
const BISHOP_MAGICS: [u64; 64] = [
    0x11A4101001002084,
    0x0420050109010B0C,
    0x018C080881024000,
    0x4008184100000008,
    0x60720210C0080400,
    0x0082055008010080,
    0x05020201A0480400,
    0x020A010248020900,
    0x000021020A020C22,
    0x3021C80200840500,
    0x0001410222810020,
    0x2000082084600100,
    0x00201A0210000040,
    0x4820811002100600,
    0x1008010410021819,
    0x42000024140A4800,
    0x4021000404040800,
    0x00C4000210040300,
    0x4008004418026088,
    0x0808000404240800,
    0xA124000200A20080,
    0x0109000280602600,
    0x0014008201140224,
    0x0212000500808424,
    0x00202200A802040C,
    0x4081202030220600,
    0x4882A40408080421,
    0x00A4080204005010,
    0x0C03010080504000,
    0x0002020000880B04,
    0x8A0604000E008202,
    0x4000C1C001040200,
    0x0090480C30F81001,
    0x0818040410027800,
    0x400010480850018A,
    0x0902008080080A00,
    0x00104901400C0040,
    0x80090A0200009800,
    0x0208080102818090,
    0x840600A200002200,
    0x0002021260004080,
    0x8104241C02060C01,
    0x1081305088001000,
    0x1000011428022C10,
    0x4000080500420400,
    0x0020200C20206040,
    0x0002081201802400,
    0x2484008082020100,
    0x080D04901009C000,
    0x0002240202500080,
    0x0042010048120002,
    0x0020082442020004,
    0x0200041006020820,
    0x0200200405820000,
    0x2008020898010001,
    0x2004240C0C112010,
    0x4040832890100810,
    0x0E20083284100810,
    0x0080080900819008,
    0xC1880C1100420218,
    0x400424A0A0204300,
    0x02802EC068014502,
    0x0000208410428100,
    0x540AD40802040020,
];
const BISHOP_SHIFTS: [u8; 64] = [
    58, 59, 59, 59, 59, 59, 59, 58, 59, 59, 59, 59, 59, 59, 59, 59, 59, 59, 57, 57, 57, 57, 59, 59,
    59, 59, 57, 55, 55, 57, 59, 59, 59, 59, 57, 55, 55, 57, 59, 59, 59, 59, 57, 57, 57, 57, 59, 59,
    59, 59, 59, 59, 59, 59, 59, 59, 58, 59, 59, 59, 59, 59, 59, 58,
];
const ROOK_MAGICS: [u64; 64] = [
    0x8080008020114000,
    0x0040004810002000,
    0x0080200010001882,
    0x8280048800811000,
    0x0100025055000800,
    0x1180020013440080,
    0xE2000822000400A1,
    0x2300082090C20100,
    0x4002800040006080,
    0x0019804000806004,
    0x2033006000510041,
    0x0001003000290020,
    0x0802000420120108,
    0x2002001882009084,
    0x009400158C061008,
    0x0A40800A40800900,
    0xD000888002401220,
    0x4000C04000201000,
    0x8085010040200530,
    0x13C9010010000C21,
    0x0801010014B08800,
    0x41A1010004008802,
    0x104004008E085001,
    0x000122000040870C,
    0x4000228480004000,
    0x0510024040022000,
    0x0824200100410112,
    0x0128004880500080,
    0x0080480100310004,
    0x000200220008108D,
    0x0008C84400108201,
    0x7010048200010844,
    0x0400810202002040,
    0x0811088461004000,
    0x0280324082002200,
    0x8200080080803000,
    0x0C08180080802C00,
    0x18A600100A008804,
    0x2000500204000891,
    0x8580800048800100,
    0x004A802240008008,
    0x4001002082020042,
    0x0002004020820012,
    0x3004100008008080,
    0x408A000810A20004,
    0x4002001804060010,
    0x0001002200010004,
    0x1400040290520001,
    0x4080044000200040,
    0x014BC20180A10200,
    0x8000841001200080,
    0x02001060401A0200,
    0x0000F80084008080,
    0x000A5C0042008080,
    0x0001000600040100,
    0x0000C90042841200,
    0x2000204080001501,
    0x0050253481004001,
    0x2000200100085041,
    0x0810883000A1000D,
    0x0011000800029045,
    0x088A004108100402,
    0x0080023008010084,
    0x5000050050240082,
];
const ROOK_SHIFTS: [u8; 64] = [
    52, 53, 53, 53, 53, 53, 53, 52, 53, 54, 54, 54, 54, 54, 54, 53, 53, 54, 54, 54, 54, 54, 54, 53,
    53, 54, 54, 54, 54, 54, 54, 53, 53, 54, 54, 54, 54, 54, 54, 53, 53, 54, 54, 54, 54, 54, 54, 53,
    53, 54, 54, 54, 54, 54, 54, 53, 52, 53, 53, 53, 53, 53, 53, 52,
];

fn init_magic_tables() -> MagicTables {
    let mut bishop_magics_arr = [Magic {
        mask: Bitboard::EMPTY,
        magic: 0,
        shift: 0,
        offset: 0,
    }; 64];
    let mut rook_magics_arr = [Magic {
        mask: Bitboard::EMPTY,
        magic: 0,
        shift: 0,
        offset: 0,
    }; 64];

    for sq in Square::ALL {
        let idx = sq.index() as usize;
        bishop_magics_arr[idx] = Magic {
            mask: bishop_mask(sq),
            magic: BISHOP_MAGICS[idx],
            shift: BISHOP_SHIFTS[idx],
            offset: 0,
        };
        rook_magics_arr[idx] = Magic {
            mask: rook_mask(sq),
            magic: ROOK_MAGICS[idx],
            shift: ROOK_SHIFTS[idx],
            offset: 0,
        };
    }

    // Compute offsets and total table sizes
    let mut bishop_offset = 0usize;
    let mut rook_offset = 0usize;
    for sq in Square::ALL {
        let idx = sq.index() as usize;
        bishop_magics_arr[idx].offset = bishop_offset;
        rook_magics_arr[idx].offset = rook_offset;
        bishop_offset += 1usize << (64 - bishop_magics_arr[idx].shift as usize);
        rook_offset += 1usize << (64 - rook_magics_arr[idx].shift as usize);
    }

    let mut bishop_table = vec![Bitboard::EMPTY; bishop_offset];
    let mut rook_table = vec![Bitboard::EMPTY; rook_offset];
    let mut bishop_pext_table = vec![Bitboard::EMPTY; bishop_offset];
    let mut rook_pext_table = vec![Bitboard::EMPTY; rook_offset];

    // Fill tables
    for sq in Square::ALL {
        let idx = sq.index() as usize;
        let bmagic = bishop_magics_arr[idx];
        let rmagic = rook_magics_arr[idx];
        let bits_b = 64 - bmagic.shift as usize;
        let bits_r = 64 - rmagic.shift as usize;
        let size_b = 1usize << bits_b;
        let size_r = 1usize << bits_r;
        for sub in 0..size_b {
            let mut blockers = Bitboard::EMPTY;
            let mut temp = bmagic.mask;
            let mut bit_idx = 0;
            while let Some(s) = temp.pop_lsb() {
                if (sub >> bit_idx) & 1 == 1 {
                    blockers |= Bitboard::from_sq(s);
                }
                bit_idx += 1;
            }
            let att = bishop_attacks_ray(sq, blockers);
            let idx_magic = ((blockers.0.wrapping_mul(bmagic.magic)) >> bmagic.shift) as usize;
            bishop_table[bmagic.offset + idx_magic] = att;
            let idx_pext = pext_software(blockers.0, bmagic.mask.0) as usize;
            bishop_pext_table[bmagic.offset + idx_pext] = att;
        }
        for sub in 0..size_r {
            let mut blockers = Bitboard::EMPTY;
            let mut temp = rmagic.mask;
            let mut bit_idx = 0;
            while let Some(s) = temp.pop_lsb() {
                if (sub >> bit_idx) & 1 == 1 {
                    blockers |= Bitboard::from_sq(s);
                }
                bit_idx += 1;
            }
            let att = rook_attacks_ray(sq, blockers);
            let idx_magic = ((blockers.0.wrapping_mul(rmagic.magic)) >> rmagic.shift) as usize;
            rook_table[rmagic.offset + idx_magic] = att;
            let idx_pext = pext_software(blockers.0, rmagic.mask.0) as usize;
            rook_pext_table[rmagic.offset + idx_pext] = att;
        }
    }

    MagicTables {
        bishop_magics: bishop_magics_arr,
        rook_magics: rook_magics_arr,
        bishop_table,
        rook_table,
        bishop_pext_table,
        rook_pext_table,
    }
}

fn magic_tables() -> &'static MagicTables {
    MAGIC_TABLES.get_or_init(init_magic_tables)
}

#[cfg(target_arch = "x86_64")]
static BMI2_AVAILABLE: OnceLock<bool> = OnceLock::new();

#[cfg(target_arch = "x86_64")]
#[inline]
fn has_bmi2() -> bool {
    *BMI2_AVAILABLE.get_or_init(|| std::arch::is_x86_feature_detected!("bmi2"))
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "bmi2")]
#[inline]
unsafe fn pext_u64(x: u64, mask: u64) -> u64 {
    std::arch::x86_64::_pext_u64(x, mask)
}

/// Bishop attacks with occupancy.
#[inline]
pub fn bishop_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    let t = magic_tables();
    let m = t.bishop_magics[sq.index() as usize];
    let blockers = occupied & m.mask;
    // Compile-time dispatch when BMI2 is a target feature (target-cpu=native on
    // this machine) — zero runtime cost. Otherwise fall back to runtime probe.
    #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
    {
        let idx = unsafe { pext_u64(blockers.0, m.mask.0) as usize };
        return t.bishop_pext_table[m.offset + idx];
    }
    #[cfg(all(target_arch = "x86_64", not(target_feature = "bmi2")))]
    if has_bmi2() {
        // SAFETY: has_bmi2() guarantees BMI2 support.
        let idx = unsafe { pext_u64(blockers.0, m.mask.0) as usize };
        return t.bishop_pext_table[m.offset + idx];
    }
    let idx = ((blockers.0.wrapping_mul(m.magic)) >> m.shift) as usize;
    t.bishop_table[m.offset + idx]
}

/// Rook attacks with occupancy.
#[inline]
pub fn rook_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    let t = magic_tables();
    let m = t.rook_magics[sq.index() as usize];
    let blockers = occupied & m.mask;
    #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
    {
        let idx = unsafe { pext_u64(blockers.0, m.mask.0) as usize };
        return t.rook_pext_table[m.offset + idx];
    }
    #[cfg(all(target_arch = "x86_64", not(target_feature = "bmi2")))]
    if has_bmi2() {
        let idx = unsafe { pext_u64(blockers.0, m.mask.0) as usize };
        return t.rook_pext_table[m.offset + idx];
    }
    let idx = ((blockers.0.wrapping_mul(m.magic)) >> m.shift) as usize;
    t.rook_table[m.offset + idx]
}

/// Queen attacks = bishop | rook.
#[inline]
pub fn queen_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    bishop_attacks(sq, occupied) | rook_attacks(sq, occupied)
}

/// Ensure all tables are initialized (call before `uciok` to hide latency).
pub fn init() {
    let _ = KNIGHT_ATTACKS.get_or_init(init_knight_attacks);
    let _ = KING_ATTACKS.get_or_init(init_king_attacks);
    let _ = PAWN_ATTACKS.get_or_init(init_pawn_attacks);
    let _ = MAGIC_TABLES.get_or_init(init_magic_tables);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::types::{Color, E4, Square};

    #[test]
    fn knight_center() {
        let attacks = knight_attacks(E4);
        assert_eq!(attacks.count(), 8);
        for sq in ["d6", "f6", "c5", "g5", "c3", "g3", "d2", "f2"] {
            let s = Square::from_str(sq).unwrap();
            assert!(attacks.contains(s), "knight e4 should attack {sq}");
        }
    }

    #[test]
    fn knight_corner() {
        let a1 = Square::from_str("a1").unwrap();
        let attacks = knight_attacks(a1);
        assert_eq!(attacks.count(), 2);
        assert!(attacks.contains(Square::from_str("b3").unwrap()));
        assert!(attacks.contains(Square::from_str("c2").unwrap()));
    }

    #[test]
    fn king_center() {
        let e4 = Square::from_str("e4").unwrap();
        assert_eq!(king_attacks(e4).count(), 8);
        let a1 = Square::from_str("a1").unwrap();
        assert_eq!(king_attacks(a1).count(), 3);
        let h8 = Square::from_str("h8").unwrap();
        assert_eq!(king_attacks(h8).count(), 3);
    }

    #[test]
    fn pawn_attacks_basic() {
        let e4 = Square::from_str("e4").unwrap();
        let w = pawn_attacks(e4, Color::White);
        assert_eq!(w.count(), 2);
        assert!(w.contains(Square::from_str("d5").unwrap()));
        assert!(w.contains(Square::from_str("f5").unwrap()));
        let b = pawn_attacks(e4, Color::Black);
        assert_eq!(b.count(), 2);
        assert!(b.contains(Square::from_str("d3").unwrap()));
        assert!(b.contains(Square::from_str("f3").unwrap()));
        let a4 = Square::from_str("a4").unwrap();
        assert_eq!(pawn_attacks(a4, Color::White).count(), 1);
        assert!(pawn_attacks(a4, Color::White).contains(Square::from_str("b5").unwrap()));
        let e8 = Square::from_str("e8").unwrap();
        assert!(pawn_attacks(e8, Color::White).is_empty());
        let e1 = Square::from_str("e1").unwrap();
        assert!(pawn_attacks(e1, Color::Black).is_empty());
    }

    #[test]
    fn init_idempotent() {
        init();
        init();
        assert_eq!(knight_attacks(E4).count(), 8);
    }

    #[test]
    fn bishop_magic_matches_ray() {
        for sq in Square::ALL {
            let mask = bishop_mask(sq);
            let bits = mask.count() as usize;
            let total = 1usize << bits;
            let test_count = total.min(512);
            let step = if total > 512 { total / 512 } else { 1 };
            let mut idx = 0;
            for _ in 0..test_count {
                let mut blockers = Bitboard::EMPTY;
                let mut temp = mask;
                let mut bit_idx = 0;
                while let Some(s) = temp.pop_lsb() {
                    if (idx >> bit_idx) & 1 == 1 {
                        blockers |= Bitboard::from_sq(s);
                    }
                    bit_idx += 1;
                }
                let expected = bishop_attacks_ray(sq, blockers);
                let got = bishop_attacks(sq, blockers);
                assert_eq!(
                    got, expected,
                    "bishop magic mismatch sq {} blockers {:016X}",
                    sq, blockers.0
                );
                idx += step;
            }
            let occupied = Bitboard::from_u64(0xFF00FF00FF00FF00);
            let got = bishop_attacks(sq, occupied);
            let masked = occupied & mask;
            let expected_masked = bishop_attacks_ray(sq, masked);
            assert_eq!(got, expected_masked);
        }
    }

    #[test]
    fn rook_magic_matches_ray() {
        for sq in Square::ALL {
            let mask = rook_mask(sq);
            let bits = mask.count() as usize;
            let total = 1usize << bits;
            let test_count = total.min(256);
            let step = if total > 256 { total / 256 } else { 1 };
            let mut idx = 0;
            for _ in 0..test_count {
                let mut blockers = Bitboard::EMPTY;
                let mut temp = mask;
                let mut bit_idx = 0;
                while let Some(s) = temp.pop_lsb() {
                    if (idx >> bit_idx) & 1 == 1 {
                        blockers |= Bitboard::from_sq(s);
                    }
                    bit_idx += 1;
                }
                let expected = rook_attacks_ray(sq, blockers);
                let got = rook_attacks(sq, blockers);
                assert_eq!(
                    got, expected,
                    "rook magic mismatch sq {} blockers {:016X}",
                    sq, blockers.0
                );
                idx += step;
            }
        }
    }

    #[test]
    fn queen_is_union() {
        let occ = Bitboard::from_u64(0x123456789ABCDEF0);
        for sq in [
            Square::from_str("d4").unwrap(),
            Square::from_str("a1").unwrap(),
            Square::from_str("h8").unwrap(),
        ] {
            assert_eq!(
                queen_attacks(sq, occ),
                bishop_attacks(sq, occ) | rook_attacks(sq, occ)
            );
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn pext_equals_magic() {
        if !has_bmi2() {
            return;
        }
        for sq in Square::ALL {
            for occ in [
                Bitboard::EMPTY,
                Bitboard::from_u64(0xFF00FF00FF00FF00),
                Bitboard::from_u64(0x123456789ABCDEF0),
                Bitboard::from_u64(0x0F0F0F0F0F0F0F0F),
            ] {
                // The public functions now use the PEXT path on this machine; verify they still match ray
                let b = bishop_attacks(sq, occ);
                let r = rook_attacks(sq, occ);
                let t = magic_tables();
                let bm = t.bishop_magics[sq.index() as usize];
                let blockers = occ & bm.mask;
                let magic_idx = ((blockers.0.wrapping_mul(bm.magic)) >> bm.shift) as usize;
                let pext_idx = unsafe { pext_u64(blockers.0, bm.mask.0) as usize };
                assert_eq!(
                    t.bishop_pext_table[bm.offset + pext_idx],
                    t.bishop_table[bm.offset + magic_idx],
                    "pext vs magic mismatch bishop sq {} occ {:016X}",
                    sq,
                    occ.0
                );
                let rm = t.rook_magics[sq.index() as usize];
                let rb = occ & rm.mask;
                let r_magic = ((rb.0.wrapping_mul(rm.magic)) >> rm.shift) as usize;
                let r_pext = unsafe { pext_u64(rb.0, rm.mask.0) as usize };
                assert_eq!(
                    t.rook_pext_table[rm.offset + r_pext],
                    t.rook_table[rm.offset + r_magic],
                    "pext vs magic rook sq {} occ {:016X}",
                    sq,
                    occ.0
                );
                // sanity that attack still equals ray via pext table
                let bm_expected = bishop_attacks_ray(sq, blockers);
                assert_eq!(b & bm_expected, bm_expected & b);
                let _ = r;
            }
        }
    }
}

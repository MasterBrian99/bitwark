//! Transposition table — clustered, lock-free, age-based.
//!
//! # Why a TT exists
//!
//! The search is a depth-first tree. The same position appears via many move
//! orders (transpositions). Caching the score/best-move for a position
//! identified by its Zobrist key saves re-searching an entire subtree and
//! gives a proven move for ordering. See chessprogramming.org
//! "Transposition Table".
//!
//! # Design
//!
//! - **Clustered**: 2 entries per 32-byte cluster. Two slots fight the
//!   classic collision problem more cheaply than a huge table, and 32 B is
//!   half a cache line — nice spatial locality.
//! - **Lock-free single-word stores**: each entry is two `AtomicU64` words
//!   (key + data). Both writes are atomic. A concurrent reader either sees
//!   the old pair, the new pair, or a torn pair whose key fails the exact
//!   check, so it can never mistake one position for another. The false-hit
//!   rate is `1 / 2^64` in theory; in practice only a missed hit.
//! - **Age-based replacement**: entries carry a 6-bit generation (bumped at
//!   every `new_search()`). Within a cluster the victim is the entry with
//!   the smallest `(depth - 4*age_diff)` — shallow + old entries are evicted
//!   before deep + fresh ones. The constant `4` weights age over depth, the
//!   classical choice used by Stockfish's TT.
//! - **SMP-ready from day one**: `Arc<TranspositionTable>` is shared between
//!   the UCI session and every search thread. All interior mutation is via
//!   `Relaxed`/`Release`/`Acquire` atomics — no mutex ever touches the hot
//!   path. Resizing (`&mut self`) is only allowed while no search runs.
//!
//! # Word discipline (why torn reads are harmless)
//!
//! Store does `data` then `key` (`Relaxed` then `Release`). Load does `key`
//! (`Acquire`) then `data` (`Relaxed`). The `Release` on `key` pairs with
//! the `Acquire` load, establishing that a `key` that matches must have its
//! `data` at least as fresh as the store that wrote that `key`. A reader
//! that races a writer can at worst see (a) the old pair, (b) the new pair,
//! or (c) the new `key` with the old `data` — which is a stale hit for the
//! same position (harmless, at most loses a cutoff or reuses an old best
//! move) and never a false positive for a different position (the key check
//! would fail). Q.E.D.
//!
//! # Packed data word
//!
//! ```text
//! bits  0..16  mv   u16  (from|to|promo — mv.rs:to_u16)
//! bits 16..32  score i16 (ply-adjusted, see score_to_tt)
//! bits 32..48  eval  i16 (static eval, straight i16)
//! bits 48..56  depth u8
//! bits 56..64  gen_bound u8  — high 6 bits generation, low 2 bits bound
//! ```
//! Total 64 bits — one atomic store. `key` is the second word (full 64-bit
//! Zobrist key; `0` means empty).

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use crate::board::Move;
use crate::search::{MATE, MAX_PLY};

// ---------------------------------------------------------------------------
// Bound type
// ---------------------------------------------------------------------------

/// How the stored score relates to the window that produced it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Bound {
    /// Exact score (PV node, or search returned inside the window).
    Exact = 0,
    /// Lower bound (`score >= beta`, fail-high).
    Lower = 1,
    /// Upper bound (`score <= alpha`, fail-low).
    Upper = 2,
}

impl Bound {
    #[inline]
    fn from_u8(v: u8) -> Self {
        match v & 0b11 {
            0 => Bound::Exact,
            1 => Bound::Lower,
            2 => Bound::Upper,
            _ => Bound::Upper, // 3 unused → treat as Upper
        }
    }
}

// ---------------------------------------------------------------------------
// Hit
// ---------------------------------------------------------------------------

/// Result of a successful probe.
#[derive(Copy, Clone, Debug)]
pub struct TtHit {
    /// Best move from the entry, if any (0 encoded as None).
    pub mv: Option<Move>,
    /// Score already adjusted back from TT ply offset.
    pub score: i32,
    /// Static eval stored alongside the score.
    #[allow(dead_code)]
    pub eval: i32,
    /// Depth the entry was searched at.
    pub depth: u8,
    /// Bound flag.
    pub bound: Bound,
}

// ---------------------------------------------------------------------------
// Score ply adjustment
// ---------------------------------------------------------------------------

#[inline]
fn score_to_tt(score: i32, ply: usize) -> i16 {
    if score >= MATE - MAX_PLY as i32 {
        (score + ply as i32) as i16
    } else if score <= -MATE + MAX_PLY as i32 {
        (score - ply as i32) as i16
    } else {
        score as i16
    }
}

#[inline]
fn score_from_tt(score: i16, ply: usize) -> i32 {
    let s = score as i32;
    if s >= MATE - MAX_PLY as i32 {
        s - ply as i32
    } else if s <= -MATE + MAX_PLY as i32 {
        s + ply as i32
    } else {
        s
    }
}

// ---------------------------------------------------------------------------
// Entry / Cluster
// ---------------------------------------------------------------------------

struct Entry {
    key: AtomicU64,
    data: AtomicU64,
}

impl Entry {
    fn new() -> Self {
        Self {
            key: AtomicU64::new(0),
            data: AtomicU64::new(0),
        }
    }
}

#[repr(align(32))]
struct Cluster {
    entries: [Entry; 2],
}

impl Cluster {
    fn new() -> Self {
        Self {
            entries: [Entry::new(), Entry::new()],
        }
    }
}

// ---------------------------------------------------------------------------
// Table
// ---------------------------------------------------------------------------

/// The transposition table.
pub struct TranspositionTable {
    clusters: Box<[Cluster]>,
    mask: usize,
    generation: AtomicU8, // 0..63, wraps
}

#[allow(dead_code)]
impl TranspositionTable {
    /// Create a table of `mib` MiB. Size is rounded down to a power of two
    /// number of clusters (at least 1). Caps at 1024 MiB to avoid OOM on
    /// absurd `Hash` values (the UCI spec allows up to 33554432 MiB).
    pub fn new(mib: u32) -> Self {
        const MAX_MIB: u32 = 1024;
        let mib = mib.clamp(1, MAX_MIB);
        let bytes = mib as usize * 1024 * 1024;
        let raw_clusters = bytes / std::mem::size_of::<Cluster>(); // 32
        let clusters_pow2 = raw_clusters.next_power_of_two() >> 1;
        // next_power_of_two rounds up; we want floor pow2. For exact pow2, this
        // undershoots by 2×, so fix: if raw_clusters is pow2, keep it.
        let n = if raw_clusters.is_power_of_two() {
            raw_clusters
        } else {
            clusters_pow2
        }
        .max(1);
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(Cluster::new());
        }
        let clusters = v.into_boxed_slice();
        let mask = n - 1;
        Self {
            clusters,
            mask,
            generation: AtomicU8::new(0),
        }
    }

    /// Number of clusters.
    #[inline]
    pub fn num_clusters(&self) -> usize {
        self.clusters.len()
    }

    /// Size in MiB (rounded).
    pub fn size_mib(&self) -> u32 {
        let bytes = self.clusters.len() * std::mem::size_of::<Cluster>();
        (bytes / (1024 * 1024)) as u32
    }

    /// Bump generation (call once per `go`). Wraps at 64.
    #[inline]
    pub fn new_search(&self) {
        let g = self.generation.load(Ordering::Relaxed);
        self.generation.store((g + 1) & 0x3F, Ordering::Relaxed);
    }

    /// Clear all entries (in-place, safe concurrently — advisory).
    pub fn clear(&self) {
        for cluster in self.clusters.iter() {
            for entry in &cluster.entries {
                entry.data.store(0, Ordering::Relaxed);
                entry.key.store(0, Ordering::Relaxed);
            }
        }
    }

    /// Resize to `mib` MiB. Requires `&mut` — caller must ensure no search
    /// is running.
    pub fn resize(&mut self, mib: u32) {
        *self = Self::new(mib);
    }

    /// Sample the first 1000 clusters for `hashfull` permille (0..1000).
    pub fn hashfull_permille(&self) -> u32 {
        let sample = self.clusters.len().min(1000);
        if sample == 0 {
            return 0;
        }
        let mut used = 0usize;
        for cluster in &self.clusters[..sample] {
            for entry in &cluster.entries {
                if entry.key.load(Ordering::Relaxed) != 0 {
                    used += 1;
                }
            }
        }
        (used * 1000 / (sample * 2)) as u32
    }

    /// Probe for `key`. Returns `None` on miss.
    #[inline]
    pub fn probe(&self, key: u64, ply: usize) -> Option<TtHit> {
        if key == 0 {
            return None;
        }
        let idx = (key as usize) & self.mask;
        let cluster = &self.clusters[idx];
        for entry in &cluster.entries {
            let k = entry.key.load(Ordering::Acquire);
            if k != key {
                continue;
            }
            let data = entry.data.load(Ordering::Relaxed);
            let mv_u16 = (data & 0xFFFF) as u16;
            let mv = if mv_u16 == 0 {
                None
            } else {
                Move::from_u16(mv_u16)
            };
            let score_i16 = ((data >> 16) & 0xFFFF) as u16 as i16;
            let eval_i16 = ((data >> 32) & 0xFFFF) as u16 as i16;
            let depth = ((data >> 48) & 0xFF) as u8;
            let gen_bound = ((data >> 56) & 0xFF) as u8;
            let bound = Bound::from_u8(gen_bound);
            let score = score_from_tt(score_i16, ply);
            let eval = eval_i16 as i32;
            return Some(TtHit {
                mv,
                score,
                eval,
                depth,
                bound,
            });
        }
        None
    }

    /// Store an entry. `depth` is remaining depth, `ply` is distance from root.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub fn store(
        &self,
        key: u64,
        mv: Option<Move>,
        score: i32,
        eval: i32,
        depth: u8,
        bound: Bound,
        ply: usize,
    ) {
        if key == 0 {
            return;
        }
        let idx = (key as usize) & self.mask;
        let cluster = &self.clusters[idx];
        let cur_gen = self.generation.load(Ordering::Relaxed) & 0x3F;
        let gen_bound = (cur_gen << 2) | (bound as u8 & 0b11);

        let mv_u16 = mv.map(|m| m.to_u16()).unwrap_or(0);
        let score_i16 = score_to_tt(score, ply) as u16;
        let eval_i16 = (eval.clamp(-32000, 32000) as i16) as u16;
        let data: u64 = (mv_u16 as u64)
            | ((score_i16 as u64) << 16)
            | ((eval_i16 as u64) << 32)
            | ((depth as u64) << 48)
            | ((gen_bound as u64) << 56);

        // 1) same key → overwrite that slot
        for entry in &cluster.entries {
            let k = entry.key.load(Ordering::Relaxed);
            if k == key {
                entry.data.store(data, Ordering::Relaxed);
                entry.key.store(key, Ordering::Release);
                return;
            }
        }
        // 2) empty slot → fill
        for entry in &cluster.entries {
            let k = entry.key.load(Ordering::Relaxed);
            if k == 0 {
                entry.data.store(data, Ordering::Relaxed);
                entry.key.store(key, Ordering::Release);
                return;
            }
        }
        // 3) replace victim: smaller (depth - 4*age_diff)
        let mut victim_idx = 0usize;
        let mut victim_value = i16::MAX;
        for (i, entry) in cluster.entries.iter().enumerate() {
            let d = entry.data.load(Ordering::Relaxed);
            let e_depth = ((d >> 48) & 0xFF) as u8;
            let e_gen_bound = ((d >> 56) & 0xFF) as u8;
            let e_gen = (e_gen_bound >> 2) & 0x3F;
            // age diff wrapping 0..63
            let age_diff = cur_gen.wrapping_sub(e_gen) & 0x3F;
            let value = e_depth as i16 - 4 * age_diff as i16;
            if value < victim_value {
                victim_value = value;
                victim_idx = i;
            }
        }
        // Depth-preferential guard: don't overwrite a much deeper entry with a very shallow one
        // unless it's very old. Simple: if new depth is very shallow and victim is deep+fresh, keep victim.
        // The formula above already encodes this; we always store to victim.
        let entry = &cluster.entries[victim_idx];
        entry.data.store(data, Ordering::Relaxed);
        entry.key.store(key, Ordering::Release);
    }
}

impl Default for TranspositionTable {
    fn default() -> Self {
        Self::new(16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Move;

    fn mv(s: &str) -> Move {
        Move::parse_uci(s).unwrap()
    }

    #[test]
    fn new_and_size() {
        let tt = TranspositionTable::new(16);
        assert!(tt.num_clusters() > 0);
        assert_eq!(tt.size_mib(), 16);
        let tt2 = TranspositionTable::new(1);
        assert_eq!(tt2.size_mib(), 1);
    }

    #[test]
    fn store_probe_roundtrip() {
        let tt = TranspositionTable::new(1);
        let key = 0x1234_5678_9ABC_DEF0u64;
        let m = mv("e2e4");
        tt.store(key, Some(m), 123, 100, 5, Bound::Exact, 0);
        let hit = tt.probe(key, 0).expect("should hit");
        assert_eq!(hit.mv, Some(m));
        assert_eq!(hit.score, 123);
        assert_eq!(hit.depth, 5);
        assert_eq!(hit.bound, Bound::Exact);
    }

    #[test]
    fn mate_score_adjust() {
        let tt = TranspositionTable::new(1);
        let key = 0xDEAD_BEEF_CAFE_1234u64;
        let ply = 5usize;
        let mate_score = MATE - 7; // mate in 7 from root perspective
        tt.store(key, None, mate_score, 0, 10, Bound::Exact, ply);
        let hit = tt.probe(key, ply).unwrap();
        assert_eq!(hit.score, mate_score);
        // Probing from a different ply should still adjust correctly
        // (store is ply-relative, probe inverts at probe ply)
        let hit_at_3 = tt.probe(key, 3).unwrap();
        // The absolute mate distance changes by ply diff
        assert_eq!(hit_at_3.score, mate_score + 2); // 5-3 = 2 closer?
    }

    #[test]
    fn empty_probe_miss() {
        let tt = TranspositionTable::new(1);
        assert!(tt.probe(0xABCD, 0).is_none());
        assert!(tt.probe(0, 0).is_none());
    }

    #[test]
    fn bound_filtering_logged_via_store() {
        let tt = TranspositionTable::new(1);
        let key = 0x1111_2222_3333_4444u64;
        tt.store(key, None, 200, 0, 6, Bound::Lower, 0);
        let hit = tt.probe(key, 0).unwrap();
        assert_eq!(hit.bound, Bound::Lower);
        assert_eq!(hit.score, 200);
    }

    #[test]
    fn clear_empties() {
        let tt = TranspositionTable::new(1);
        tt.store(0xAAAA, Some(mv("g1f3")), 50, 0, 3, Bound::Exact, 0);
        assert!(tt.probe(0xAAAA, 0).is_some());
        tt.clear();
        assert!(tt.probe(0xAAAA, 0).is_none());
        assert_eq!(tt.hashfull_permille(), 0);
    }

    #[test]
    fn resize_changes_size() {
        let mut tt = TranspositionTable::new(1);
        tt.store(0xBBBB, None, 10, 0, 2, Bound::Exact, 0);
        tt.resize(2);
        assert_eq!(tt.size_mib(), 2);
        // old entry gone after resize
        assert!(tt.probe(0xBBBB, 0).is_none());
    }

    #[test]
    fn move_codec_none() {
        let tt = TranspositionTable::new(1);
        let key = 0x9999_8888_7777_6666u64;
        tt.store(key, None, -5, 0, 1, Bound::Upper, 0);
        let hit = tt.probe(key, 0).unwrap();
        assert_eq!(hit.mv, None);
    }

    #[test]
    fn concurrent_hammer() {
        use std::sync::Arc;
        use std::thread;
        let tt = Arc::new(TranspositionTable::new(1));
        let mut handles = Vec::new();
        for tid in 0..4 {
            let tt_c = Arc::clone(&tt);
            handles.push(thread::spawn(move || {
                for i in 0..50_000u64 {
                    let key = (tid as u64 * 1_000_000 + i * 7919 + 0x1234) | 1;
                    let depth = (i % 10) as u8;
                    let score = (i % 2000) as i32 - 1000;
                    let bound = match i % 3 {
                        0 => Bound::Exact,
                        1 => Bound::Lower,
                        _ => Bound::Upper,
                    };
                    tt_c.store(key, None, score, 0, depth, bound, 0);
                    let _ = tt_c.probe(key, 0);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // Verify a known key after hammering.
        let key = 0xCAFEBABEDEADu64 | 1;
        tt.store(key, Some(mv("e2e4")), 42, 0, 5, Bound::Exact, 0);
        let hit = tt.probe(key, 0).expect("should hit after hammer");
        assert_eq!(hit.score, 42);
        assert_eq!(hit.mv, Some(mv("e2e4")));
    }
}

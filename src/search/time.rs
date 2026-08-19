//! Time management — soft/hard limits, Move Overhead, pondering.
//!
//! The search uses two budgets per move:
//!
//! * **soft** — optimum time. The search will not *start* a new iteration
//!   once `elapsed > soft`. The current iteration is always finished (so the
//!   last completed result is returned).
//! * **hard** — maximum time. Checked inside the search every 2048 nodes; if
//!   `elapsed >= hard` the search is aborted immediately (best-effort `bestmove`
//!   from the last completed iteration is returned).
//!
//! Classical formulas (see chessprogramming.org "Time Management"):
//! ```text
//! avail = remaining.saturating_sub(overhead)        // Move Overhead = assumed latency
//! if avail == 0            -> soft 5ms, hard 10ms   // must still move, never flag
//! else if movestogo > 0:  soft = avail/movestogo + 3*inc/4
//!                         hard = min(avail/2, 4*soft)
//! else (sudden death):     soft = avail/30      + 3*inc/4
//!                         hard = min(avail/5, 5*soft)
//! floors: soft ≥ 5ms; hard = clamp(hard, soft, avail)
//! ```
//! `movetime` maps to `soft = hard = ms` (exact-ish). No clock at all
//! (`go depth`/`go infinite`/bare `go`) → `soft = hard = 0` (no limit).
//! All constants are documented and tunable here.
//!
//! Pondering: while `pondering == true` both soft/hard checks return false.
//! `ponderhit()` records `start = now` and clears the flag — the clock then
//! starts counting. `start_pondering()` is used by the session for
//! `go ponder` (budget already computed, clock not yet started).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

/// Global epoch for `AtomicU64` millisecond timestamps. Lazily initialized on
/// first use; subsequent `now_ms()` calls return `elapsed.as_millis()` since
/// this instant.
static EPOCH: OnceLock<Instant> = OnceLock::new();

#[inline]
fn now_ms() -> u64 {
    let epoch = EPOCH.get_or_init(Instant::now);
    Instant::now().duration_since(*epoch).as_millis() as u64
}

/// Shared, ponder-aware clock for one `go`.
///
/// `soft_ms`/`hard_ms` are immutable after construction; `start_ms` and
/// `pondering` are atomically mutated on `ponderhit()` so the search thread
/// (hot path, every 2048 nodes) and the session task (async) can share one
/// `Arc<TimeControl>` without a mutex.
pub struct TimeControl {
    soft_ms: u64,
    hard_ms: u64,
    start_ms: AtomicU64,
    pondering: AtomicBool,
}

impl std::fmt::Debug for TimeControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimeControl")
            .field("soft_ms", &self.soft_ms)
            .field("hard_ms", &self.hard_ms)
            .field("elapsed_ms", &self.elapsed_ms())
            .field("pondering", &self.pondering.load(Ordering::Relaxed))
            .finish()
    }
}

impl TimeControl {
    /// No time limit. `should_*_stop()` always returns false.
    pub fn none() -> Self {
        Self {
            soft_ms: 0,
            hard_ms: 0,
            start_ms: AtomicU64::new(now_ms()),
            pondering: AtomicBool::new(false),
        }
    }

    /// Exact wall-clock budget: `soft == hard == ms`. Clock starts now.
    pub fn for_movetime(ms: u64) -> Self {
        let ms = ms.max(5);
        Self {
            soft_ms: ms,
            hard_ms: ms,
            start_ms: AtomicU64::new(now_ms()),
            pondering: AtomicBool::new(false),
        }
    }

    /// Sudden-death or repeating time control.
    ///
    /// * `remaining_ms` — our remaining clock (`wtime`/`btime`).
    /// * `inc_ms` — increment per move (`winc`/`binc`, 0 if absent).
    /// * `movestogo` — moves until next time control (`None`/`Some(0)` = sudden death).
    /// * `overhead_ms` — `Move Overhead` option (assumed latency, subtracted).
    pub fn for_clock(
        remaining_ms: u64,
        inc_ms: u64,
        movestogo: Option<u32>,
        overhead_ms: u32,
    ) -> Self {
        let avail = remaining_ms.saturating_sub(overhead_ms as u64);
        if avail == 0 {
            return Self {
                soft_ms: 5,
                hard_ms: 10,
                start_ms: AtomicU64::new(now_ms()),
                pondering: AtomicBool::new(false),
            };
        }

        let (mut soft, mut hard) = match movestogo {
            Some(m) if m > 0 => {
                let m = m as u64;
                let soft = avail / m + inc_ms * 3 / 4;
                let hard = (avail / 2).min(soft * 4);
                (soft, hard)
            }
            _ => {
                let soft = avail / 30 + inc_ms * 3 / 4;
                let hard = (avail / 5).min(soft * 5);
                (soft, hard)
            }
        };

        // Floors and cap.
        if soft < 5 {
            soft = 5;
        }
        if hard < soft {
            hard = soft;
        }
        if hard > avail {
            hard = avail;
        }
        if soft > avail {
            soft = avail;
        }
        // Hard must be at least 10ms so we always make a move on tiny clocks.
        if hard < 10 {
            hard = 10;
            if soft > hard {
                soft = hard;
            }
        }

        Self {
            soft_ms: soft,
            hard_ms: hard,
            start_ms: AtomicU64::new(now_ms()),
            pondering: AtomicBool::new(false),
        }
    }

    /// Mark this control as "pondering": budget computed, but clock not yet
    /// started. While pondering, `should_*_stop()` return false.
    pub fn start_pondering(&self) {
        self.pondering.store(true, Ordering::Relaxed);
        // 0 = not yet started; elapsed() will return 0 while pondering.
        self.start_ms.store(0, Ordering::Relaxed);
    }

    /// Called by the session when `ponderhit` arrives. Starts the clock now
    /// and clears the pondering flag. No-op if not pondering.
    pub fn ponderhit(&self) {
        if self.pondering.load(Ordering::Relaxed) {
            self.start_ms.store(now_ms(), Ordering::Relaxed);
            self.pondering.store(false, Ordering::Relaxed);
        }
    }

    /// True while `go ponder` is active and `ponderhit` has not yet arrived.
    pub fn is_pondering(&self) -> bool {
        self.pondering.load(Ordering::Relaxed)
    }

    /// Milliseconds since the clock started (0 while pondering before hit).
    pub fn elapsed_ms(&self) -> u64 {
        if self.is_pondering() {
            return 0;
        }
        let start = self.start_ms.load(Ordering::Relaxed);
        now_ms().saturating_sub(start)
    }

    /// Soft limit reached: don't start a new iteration.
    pub fn should_soft_stop(&self) -> bool {
        if self.is_pondering() {
            return false;
        }
        if self.soft_ms == 0 {
            return false;
        }
        self.elapsed_ms() >= self.soft_ms
    }

    /// Hard limit reached: abort immediately (checked every 2048 nodes).
    pub fn should_hard_stop(&self) -> bool {
        if self.is_pondering() {
            return false;
        }
        if self.hard_ms == 0 {
            return false;
        }
        self.elapsed_ms() >= self.hard_ms
    }

    /// Direct accessors for the soft/hard values (tests, ID-loop heuristic).
    pub fn soft_ms(&self) -> u64 {
        self.soft_ms
    }
    pub fn hard_ms(&self) -> u64 {
        self.hard_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn none_never_stops() {
        let tc = TimeControl::none();
        assert!(!tc.should_soft_stop());
        assert!(!tc.should_hard_stop());
        assert_eq!(tc.elapsed_ms(), 0);
        // Even after a short sleep, still no stop (soft==hard==0).
        thread::sleep(Duration::from_millis(5));
        assert!(!tc.should_soft_stop());
    }

    #[test]
    fn movetime_soft_eq_hard() {
        let tc = TimeControl::for_movetime(100);
        assert_eq!(tc.soft_ms(), 100);
        assert_eq!(tc.hard_ms(), 100);
        assert!(!tc.should_hard_stop());
        // Sleep well beyond budget; avoid flakiness on loaded CI boxes.
        thread::sleep(Duration::from_millis(200));
        assert!(tc.should_hard_stop());
        assert!(tc.should_soft_stop());
    }

    #[test]
    fn clock_sudden_death_budgets() {
        // 60s + 1s inc, sudden death, overhead 10 → avail 59990
        // soft = 59990/30 + 750 = 1999+750=2749, hard = min(11998, 13745)=11998
        let tc = TimeControl::for_clock(60000, 1000, None, 10);
        assert_eq!(tc.soft_ms(), 2749);
        assert_eq!(tc.hard_ms(), 11998);
        assert!(tc.hard_ms() <= 59990);
        assert!(tc.soft_ms() <= tc.hard_ms());
    }

    #[test]
    fn clock_movestogo_budgets() {
        // 60s, no inc, movestogo 40, overhead 0 → avail 60000
        // soft = 1500, hard = min(30000, 6000)=6000
        let tc = TimeControl::for_clock(60000, 0, Some(40), 0);
        assert_eq!(tc.soft_ms(), 1500);
        assert_eq!(tc.hard_ms(), 6000);
    }

    #[test]
    fn clock_tiny_remaining_floor() {
        // 20ms remaining, overhead 0 → avail 20 → floors kick in
        let tc = TimeControl::for_clock(20, 0, None, 0);
        assert!(tc.soft_ms() >= 5);
        assert!(tc.hard_ms() >= tc.soft_ms());
        assert!(tc.hard_ms() <= 20);
    }

    #[test]
    fn clock_zero_avail_floor() {
        // remaining 10, overhead 10 → avail 0 → 5/10 floor
        let tc = TimeControl::for_clock(10, 0, None, 10);
        assert_eq!(tc.soft_ms(), 5);
        assert_eq!(tc.hard_ms(), 10);
    }

    #[test]
    fn clock_overhead_subtracted() {
        // 1000ms, inc 0, overhead 100 → avail 900
        // sudden death soft = 900/30 = 30, hard = min(180,150)=150
        let tc = TimeControl::for_clock(1000, 0, None, 100);
        assert_eq!(tc.soft_ms(), 30);
        assert_eq!(tc.hard_ms(), 150);
        // Without overhead soft would be 33 → overhead matters.
        let tc2 = TimeControl::for_clock(1000, 0, None, 0);
        assert!(tc2.soft_ms() > tc.soft_ms());
    }

    #[test]
    fn pondering_gates_stops() {
        let tc = TimeControl::for_movetime(50);
        tc.start_pondering();
        assert!(tc.is_pondering());
        assert_eq!(tc.elapsed_ms(), 0);
        assert!(!tc.should_hard_stop());
        assert!(!tc.should_soft_stop());
        // After ponderhit clock starts; stops are armed again.
        tc.ponderhit();
        assert!(!tc.is_pondering());
        // Second ponderhit is a no-op.
        let start = tc.start_ms.load(Ordering::Relaxed);
        tc.ponderhit();
        assert_eq!(tc.start_ms.load(Ordering::Relaxed), start);
        thread::sleep(Duration::from_millis(60));
        assert!(tc.should_hard_stop());
    }

    #[test]
    fn hard_le_avail_invariant() {
        for &rem in &[50, 100, 1000, 60000] {
            for &inc in &[0, 100, 1000] {
                for &mtg in &[None, Some(1), Some(40)] {
                    for &oh in &[0, 10, 100] {
                        let tc = TimeControl::for_clock(rem, inc, mtg, oh);
                        let avail = rem.saturating_sub(oh as u64);
                        if avail > 0 {
                            assert!(
                                tc.hard_ms() <= avail,
                                "hard {} > avail {} (rem {rem} inc {inc} mtg {mtg:?} oh {oh})",
                                tc.hard_ms(),
                                avail
                            );
                        }
                        assert!(tc.soft_ms() <= tc.hard_ms());
                        assert!(tc.soft_ms() >= 5 || avail < 5);
                    }
                }
            }
        }
    }
}

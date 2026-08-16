//! The engine's UCI option registry.
//!
//! `uci` must advertise every configurable knob as a typed `option` line
//! (UCI spec §3.5), and `setoption` from the GUI updates them. This struct is
//! the single source of truth: `option_lines()` generates the advertised
//! list, and `set()` applies incoming changes — including clamping `spin`
//! values into `[min, max]` as well-behaved engines should.
//!
//! Several options are declared ahead of the code that consumes them
//! ; those fields are stored but not yet read, hence the
//! `#[allow(dead_code)]`.

/// Tunable engine configuration. Mutated only through `setoption`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct EngineOptions {
    /// Search threads (Lazy SMP). Consumed in Phase 8; locked to 1 until then.
    pub threads: u32,
    /// Transposition-table size, MiB.
    pub hash_mib: u32,
    /// Pondering enabled.
    pub ponder: bool,
    /// Milliseconds subtracted from our clock per move for GUI/network latency.
    pub move_overhead_ms: u32,
    /// Number of principal variations reported.
    pub multipv: u32,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            threads: 1,
            hash_mib: 16,
            ponder: false,
            move_overhead_ms: 10,
            multipv: 1,
        }
    }
}

impl EngineOptions {
    /// The `option name ...` lines sent between `id author` and `uciok`
    /// (UCI spec §2.1 / §3.5). One line per configurable knob, in the exact
    /// grammar GUIs parse by keyword.
    pub fn option_lines(&self) -> Vec<String> {
        vec![
            "option name Threads type spin default 1 min 1 max 1024".into(),
            "option name Hash type spin default 16 min 1 max 33554432".into(),
            "option name Clear Hash type button".into(),
            "option name Ponder type check default false".into(),
            "option name Move Overhead type spin default 10 min 0 max 5000".into(),
            "option name MultiPV type spin default 1 min 1 max 500".into(),
        ]
    }

    /// Apply one `setoption`. Returns `true` when the option name was
    /// recognized. Unknown names are silently ignored per UCI spec §2.4.
    ///
    /// Spin values are clamped into `[min, max]` (well-behaved-engine
    /// behaviour); malformed values leave the option unchanged.
    pub fn set(&mut self, name: &str, value: Option<&str>) -> bool {
        match name {
            "Threads" => self.threads = clamp(value, 1, 1024, self.threads),
            "Hash" => self.hash_mib = clamp(value, 1, 33554432, self.hash_mib),
            "Ponder" => {
                if let Some(b) = parse_bool(value) {
                    self.ponder = b;
                }
            }
            "Move Overhead" => {
                self.move_overhead_ms = clamp(value, 0, 5000, self.move_overhead_ms);
            }
            "MultiPV" => self.multipv = clamp(value, 1, 500, self.multipv),
            // Button: stateless trigger; clears the transposition table.
            "Clear Hash" => {}
            _ => return false,
        }
        true
    }
}

/// Parse `value` as a `spin` integer and clamp it into `[min, max]`,
/// returning `current` when the value is missing or malformed.
fn clamp(value: Option<&str>, min: u32, max: u32, current: u32) -> u32 {
    value
        .and_then(|v| v.parse::<i64>().ok())
        .map(|v| v.clamp(min as i64, max as i64) as u32)
        .unwrap_or(current)
}

/// Parse `check`-option values. GUIs send `true`/`false`; some also send
/// `1`/`0`.
fn parse_bool(value: Option<&str>) -> Option<bool> {
    match value? {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setoption_clamps_and_parses() {
        let mut o = EngineOptions::default();
        assert!(o.set("Threads", Some("4")));
        assert_eq!(o.threads, 4);

        assert!(o.set("Threads", Some("999999")));
        assert_eq!(o.threads, 1024);

        assert!(o.set("Threads", Some("banana")));
        assert_eq!(o.threads, 1024);

        assert!(o.set("Move Overhead", Some("100")));
        assert_eq!(o.move_overhead_ms, 100);

        assert!(o.set("Clear Hash", None));

        assert!(o.set("Ponder", Some("true")));
        assert!(o.ponder);

        assert!(!o.set("SyzygyPath", Some("/tb")));
    }
}

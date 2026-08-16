//! The UCI (Universal Chess Interface) protocol layer.
//!
//! UCI in one paragraph: the engine is a stateless-looking line protocol —
//! the GUI sets a position with `position`, starts thinking with `go`, and
//! the engine streams `info` progress lines until it emits exactly one
//! `bestmove`. The modules
//! here follow its section numbering.
//!
//! * `parse`   — text line → `UciCommand` (pure function, trivially testable)
//! * `options` — the `setoption` registry and the engine's tunables
//! * `session` — the async command loop + stdout writer task

pub mod options;
pub mod parse;
pub mod session;

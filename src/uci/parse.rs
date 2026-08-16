//! Turning UCI text lines into typed commands.
//!
//! UCI is a plain-ASCII, line-oriented protocol: the GUI sends one command
//! per `\n`-terminated line and the engine replies with one-line responses
//! (UCI spec §1). Parsing is therefore a pure function from `&str` to a
//! typed `UciCommand`.
//!
//! Two rules from the spec drive the design:
//!
//! 1. **Forward compatibility.** Unknown commands, and unknown tokens
//!    inside known commands, must be ignored rather than rejected
//!    (UCI spec §1). `parse_line` never returns an error; the worst case is
//!    `UciCommand::Unknown`.
//! 2. **Verbatim option/position data.** `setoption name <id>` and
//!    `position fen <fenstring>` contain spaces in the middle of an
//!    argument, so naive whitespace splitting is not enough.

// ----------------------------------------------------------------------------

/// Every limit that can ride on a `go` command (UCI spec §2.8).
///
/// All fields are optional because the GUI may send any subset; the search
/// combines them with "first limit hit wins" semantics.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GoParams {
    /// Restrict the root search to these moves only.
    pub searchmoves: Vec<String>,
    /// Search on the anticipated reply (pondering).
    pub ponder: bool,
    /// White's remaining clock, milliseconds.
    pub wtime: Option<u64>,
    /// Black's remaining clock, milliseconds.
    pub btime: Option<u64>,
    /// White's increment per move, milliseconds.
    pub winc: Option<u64>,
    /// Black's increment per move, milliseconds.
    pub binc: Option<u64>,
    /// Moves left until the next time control.
    pub movestogo: Option<u32>,
    /// Fixed search depth in plies.
    pub depth: Option<u8>,
    /// Approximate node-count limit.
    pub nodes: Option<u64>,
    /// Search for a mate in `mate` moves.
    pub mate: Option<u8>,
    /// Search for exactly this many milliseconds.
    pub movetime: Option<u64>,
    /// Search until `stop` — never stop on our own.
    pub infinite: bool,
    /// Debug limit: run a perft (raw movegen node count) to this depth.
    pub perft: Option<u8>,
}

/// Keywords that introduce a `go` limit. Used to know where the
/// `searchmoves` move list ends.
const GO_KEYWORDS: [&str; 13] = [
    "searchmoves",
    "ponder",
    "wtime",
    "btime",
    "winc",
    "binc",
    "movestogo",
    "depth",
    "nodes",
    "mate",
    "movetime",
    "infinite",
    "perft",
];

/// A parsed UCI command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UciCommand {
    /// `uci` — switch to UCI mode; reply with id/options/uciok.
    Uci,
    /// `debug on|off` — toggle extra `info string` output.
    Debug(bool),
    /// `isready` — always answer `readyok`, even mid-search.
    IsReady,
    /// `setoption name <id> [value <x>]`.
    SetOption { name: String, value: Option<String> },
    /// `ucinewgame` — the next position starts a fresh game.
    UciNewGame,
    /// `position [fen <fen>|startpos] [moves ...]`.
    /// `fen == None` means the standard starting position.
    Position {
        fen: Option<String>,
        moves: Vec<String>,
    },
    /// `go [limits...]`.
    Go(GoParams),
    /// `stop` — interrupt the search as soon as possible.
    Stop,
    /// `ponderhit` — the pondered move was played.
    PonderHit,
    /// `quit` — exit immediately.
    Quit,
    /// `register ...` — accepted and ignored (no registration scheme).
    Register,
    /// Anything unrecognized, malformed, or empty. Per the protocol these
    /// must be silently ignored so the engine stays forward-compatible with
    /// GUIs speaking newer UCI dialects.
    Unknown,
}

/// Parse one UCI command line. Never panics; never returns an error.
pub fn parse_line(line: &str) -> UciCommand {
    let mut tokens = line.split_whitespace();
    let first = match tokens.next() {
        Some(t) => t,
        None => return UciCommand::Unknown, // empty line
    };
    let rest: Vec<&str> = tokens.collect();
    match first {
        "uci" => UciCommand::Uci,
        "isready" => UciCommand::IsReady,
        "ucinewgame" => UciCommand::UciNewGame,
        "stop" => UciCommand::Stop,
        "ponderhit" => UciCommand::PonderHit,
        "quit" => UciCommand::Quit,
        "register" => UciCommand::Register,
        "debug" => match rest.first() {
            Some(&"on") => UciCommand::Debug(true),
            Some(&"off") => UciCommand::Debug(false),
            _ => UciCommand::Unknown,
        },
        "setoption" => parse_setoption(&rest),
        "position" => parse_position(&rest),
        "go" => UciCommand::Go(parse_go(&rest)),
        _ => UciCommand::Unknown,
    }
}

/// `setoption name <id> [value <x>]`
///
/// The option name may contain spaces ("Clear Hash", "Move Overhead"), so
/// it is everything between the `name` keyword and the first `value`
/// keyword (or end of line). The value, when present, is everything after
/// `value`, rejoined with spaces (UCI spec §2.4).
fn parse_setoption(rest: &[&str]) -> UciCommand {
    if rest.first() != Some(&"name") {
        return UciCommand::Unknown;
    }
    let rest = &rest[1..];

    let (name_toks, value_toks) = match rest.iter().position(|&t| t == "value") {
        Some(vi) => (&rest[..vi], Some(&rest[vi + 1..])),
        None => (rest, None),
    };

    if name_toks.is_empty() {
        return UciCommand::Unknown;
    }

    UciCommand::SetOption {
        name: name_toks.join(" "),
        value: value_toks.map(|v| v.join(" ")),
    }
}

/// `position [fen <fenstring> | startpos] [moves <move1> ... <movei>]`
///
/// `fen` is stored as the verbatim string of FEN tokens; validated and
/// decoded when the command runs. `fen == None` means the starting position.
fn parse_position(rest: &[&str]) -> UciCommand {
    let mut fen: Option<String> = None;
    let mut moves: Vec<String> = Vec::new();

    let mut i;
    match rest.first() {
        Some(&"startpos") => i = 1,
        Some(&"fen") => {
            i = 1;
            let mut fields: Vec<&str> = Vec::new();
            while i < rest.len() && rest[i] != "moves" {
                fields.push(rest[i]);
                i += 1;
            }
            if fields.is_empty() {
                return UciCommand::Unknown;
            }
            fen = Some(fields.join(" "));
        }
        _ => return UciCommand::Unknown,
    }

    if rest.get(i) == Some(&"moves") {
        for &m in &rest[i + 1..] {
            moves.push(m.to_string());
        }
    }

    UciCommand::Position { fen, moves }
}

/// `go [searchmoves ...] [ponder] [wtime x] [btime x] ...`
///
/// Limits are combined; the engine must stop at whichever fires first.
fn parse_go(rest: &[&str]) -> GoParams {
    let mut p = GoParams::default();
    let mut i = 0;
    while i < rest.len() {
        let kw = rest[i];

        // Flag parameters carry no value token.
        if kw == "ponder" {
            p.ponder = true;
            i += 1;
            continue;
        }
        if kw == "infinite" {
            p.infinite = true;
            i += 1;
            continue;
        }
        if kw == "searchmoves" {
            // Move list runs until the next `go` keyword or end of line.
            i += 1;
            while i < rest.len() && !GO_KEYWORDS.contains(&rest[i]) {
                p.searchmoves.push(rest[i].to_string());
                i += 1;
            }
            continue;
        }

        // Value parameters: keyword immediately followed by a number.
        // A missing/garbage/overflowing number makes the whole parameter
        // ignored; the search later falls back to sensible defaults.
        let val = rest.get(i + 1).copied().and_then(|t| t.parse::<u64>().ok());
        let consumed_pair = match (kw, val) {
            ("wtime", Some(v)) => {
                p.wtime = Some(v);
                true
            }
            ("btime", Some(v)) => {
                p.btime = Some(v);
                true
            }
            ("winc", Some(v)) => {
                p.winc = Some(v);
                true
            }
            ("binc", Some(v)) => {
                p.binc = Some(v);
                true
            }
            ("movestogo", Some(v)) if v <= u32::MAX as u64 => {
                p.movestogo = Some(v as u32);
                true
            }
            ("depth", Some(v)) if v <= u8::MAX as u64 => {
                p.depth = Some(v as u8);
                true
            }
            ("nodes", Some(v)) => {
                p.nodes = Some(v);
                true
            }
            ("mate", Some(v)) if v <= u8::MAX as u64 => {
                p.mate = Some(v as u8);
                true
            }
            ("movetime", Some(v)) => {
                p.movetime = Some(v);
                true
            }
            ("perft", Some(v)) if v <= u8::MAX as u64 => {
                p.perft = Some(v as u8);
                true
            }
            _ => false,
        };

        i += if consumed_pair { 2 } else { 1 };
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_commands() {
        assert_eq!(parse_line("uci"), UciCommand::Uci);
        assert_eq!(parse_line("isready"), UciCommand::IsReady);
        assert_eq!(parse_line("ucinewgame"), UciCommand::UciNewGame);
        assert_eq!(parse_line("stop"), UciCommand::Stop);
        assert_eq!(parse_line("ponderhit"), UciCommand::PonderHit);
        assert_eq!(parse_line("quit"), UciCommand::Quit);
        assert_eq!(parse_line("register later"), UciCommand::Register);
    }

    #[test]
    fn whitespace_and_crlf_tolerated() {
        assert_eq!(parse_line("  uci  "), UciCommand::Uci);
        assert_eq!(parse_line("uci\r\n"), UciCommand::Uci);
        assert_eq!(parse_line(""), UciCommand::Unknown);
        assert_eq!(parse_line("   "), UciCommand::Unknown);
    }

    #[test]
    fn unknown_ignored() {
        assert_eq!(parse_line("this is nonsense"), UciCommand::Unknown);
        assert_eq!(parse_line("uciextra"), UciCommand::Unknown);
    }

    #[test]
    fn debug_flag() {
        assert_eq!(parse_line("debug on"), UciCommand::Debug(true));
        assert_eq!(parse_line("debug off"), UciCommand::Debug(false));
        assert_eq!(parse_line("debug sideways"), UciCommand::Unknown);
        assert_eq!(parse_line("debug"), UciCommand::Unknown);
    }

    #[test]
    fn setoption_names_with_spaces() {
        assert_eq!(
            parse_line("setoption name Clear Hash"),
            UciCommand::SetOption {
                name: "Clear Hash".into(),
                value: None
            }
        );
        assert_eq!(
            parse_line("setoption name Move Overhead value 100"),
            UciCommand::SetOption {
                name: "Move Overhead".into(),
                value: Some("100".into())
            }
        );
        assert_eq!(
            parse_line("setoption name Threads value 4"),
            UciCommand::SetOption {
                name: "Threads".into(),
                value: Some("4".into())
            }
        );
        assert_eq!(parse_line("setoption Threads value 4"), UciCommand::Unknown);
        assert_eq!(parse_line("setoption name"), UciCommand::Unknown);
    }

    #[test]
    fn position_flavours() {
        assert_eq!(
            parse_line("position startpos"),
            UciCommand::Position {
                fen: None,
                moves: vec![]
            }
        );
        assert_eq!(
            parse_line("position startpos moves e2e4 e7e5 g1f3"),
            UciCommand::Position {
                fen: None,
                moves: vec!["e2e4".into(), "e7e5".into(), "g1f3".into()]
            }
        );
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert_eq!(
            parse_line(&format!("position fen {fen}")),
            UciCommand::Position {
                fen: Some(fen.into()),
                moves: vec![]
            }
        );
        assert_eq!(
            parse_line(&format!("position fen {fen} moves e2e4 c7c5")),
            UciCommand::Position {
                fen: Some(fen.into()),
                moves: vec!["e2e4".into(), "c7c5".into()]
            }
        );
        assert_eq!(parse_line("position bogus"), UciCommand::Unknown);
    }

    #[test]
    fn go_limits() {
        let p = |line: &str| match parse_line(line) {
            UciCommand::Go(p) => p,
            other => panic!("expected Go, got {other:?}"),
        };
        assert_eq!(p("go"), GoParams::default());

        let g = p("go depth 5 movetime 1500");
        assert_eq!(g.depth, Some(5));
        assert_eq!(g.movetime, Some(1500));

        let g = p("go wtime 60000 btime 55000 winc 1000 binc 1000 movestogo 40");
        assert_eq!(g.wtime, Some(60000));
        assert_eq!(g.btime, Some(55000));
        assert_eq!(g.winc, Some(1000));
        assert_eq!(g.binc, Some(1000));
        assert_eq!(g.movestogo, Some(40));

        let g = p("go infinite ponder mate 2 nodes 50000 perft 4");
        assert!(g.infinite && g.ponder);
        assert_eq!(g.mate, Some(2));
        assert_eq!(g.nodes, Some(50000));
        assert_eq!(g.perft, Some(4));

        let g = p("go searchmoves e2e4 d2d4 depth 3");
        assert_eq!(g.searchmoves, vec!["e2e4".to_string(), "d2d4".to_string()]);
        assert_eq!(g.depth, Some(3));

        // Unknown parameters and garbage values never swallow real keywords.
        let g = p("go bogus 7 depth 3");
        assert_eq!(g.depth, Some(3));
        let g = p("go depth banana");
        assert_eq!(g.depth, None);
    }
}

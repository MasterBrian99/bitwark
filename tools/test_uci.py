"""
UCI handshake conformance.

Checks the protocol shell against the UCI spec:
  §2.1  `uci` -> id name / id author / option lines / uciok
  §2.2  `debug on|off` accepted silently
  §2.3  `isready` -> `readyok` promptly
  §2.4  `setoption` (known, unknown, out-of-range) accepted silently
  §2.5  `register ...` ignored
  §2.7  `position ...` accepted silently (inert until the board lands)
  §2.6  `ucinewgame` accepted
  §2.11 `quit` terminates the process

Plus: no unsolicited output at startup, engine survives unknown commands.

Run:  cd tools && uv run python test_uci.py
"""

import asyncio

from common import BITWARK_BIN, UciEngine

VALID_OPTION_TYPES = {"check", "spin", "combo", "button", "string"}


class Failure(AssertionError):
    """A failed conformance check."""


def check(cond: bool, msg: str) -> None:
    if not cond:
        raise Failure(msg)


async def test_no_startup_output(eng: UciEngine) -> None:
    # A compliant engine is silent until spoken to (UCI spec §1).
    got = await eng.read_silence(0.3)
    check(not got, f"unsolicited output at startup: {got}")


async def test_uci_handshake(eng: UciEngine) -> None:
    # `uci` must be repeatable and stateless.
    for _ in range(2):
        lines = await eng.handshake()
        check(len(lines) >= 3, f"need >= 3 lines (id, id, uciok), got {lines}")
        check(lines[0].startswith("id name Bitwark"), f"bad id name: {lines[0]!r}")
        check(lines[1].startswith("id author "), f"bad id author: {lines[1]!r}")
        check(lines[-1] == "uciok", f"must end with uciok, got {lines[-1]!r}")
        for line in lines[2:-1]:
            check_option_line(line)


def check_option_line(line: str) -> None:
    """Validate one `option name ... type ...` declaration (UCI spec §3.5)."""
    toks = line.split()
    check(toks[:2] == ["option", "name"], f"bad option prefix: {line!r}")
    check("type" in toks, f"option missing type keyword: {line!r}")
    ti = toks.index("type")
    otype = toks[ti + 1]
    check(otype in VALID_OPTION_TYPES, f"invalid option type {otype!r}: {line!r}")
    if otype == "spin":
        for kw in ("default", "min", "max"):
            check(kw in toks, f"spin option missing {kw}: {line!r}")
        di = toks.index("default")
        check(toks[di + 1].lstrip("-").isdigit(), f"non-numeric default: {line!r}")


async def test_isready(eng: UciEngine) -> None:
    lines = await eng.isready()
    check(lines == ["readyok"], f"expected exactly ['readyok'], got {lines}")


async def test_debug(eng: UciEngine) -> None:
    await eng.send("debug on")
    await eng.send("debug off")
    got = await eng.read_silence(0.2)
    check(not got, f"debug must not produce output yet: {got}")


async def test_register(eng: UciEngine) -> None:
    await eng.send("register later")
    await eng.send("register name Someone code 1234")
    got = await eng.read_silence(0.2)
    check(not got, f"registration must be ignored silently: {got}")


async def test_unknown_commands(eng: UciEngine) -> None:
    await eng.send("definitely not a command")
    await eng.send("42")
    got = await eng.read_silence(0.2)
    check(not got, f"unknown commands must be ignored: {got}")
    # ...and the engine must still be alive and responsive:
    lines = await eng.isready()
    check(lines[-1] == "readyok", f"engine unresponsive after unknown input: {lines}")


async def test_setoption(eng: UciEngine) -> None:
    await eng.send("setoption name Threads value 4")
    await eng.send("setoption name Hash value 64")
    await eng.send("setoption name Move Overhead value 100")
    await eng.send("setoption name Clear Hash")
    await eng.send("setoption name No Such Option value 1")  # ignored
    await eng.send("setoption name Hash value 999999999999")  # clamped, no error
    got = await eng.read_silence(0.2)
    check(not got, f"setoption must never print anything: {got}")
    lines = await eng.isready()
    check(lines[-1] == "readyok", f"engine unresponsive after setoptions: {lines}")


async def test_position_accepted(eng: UciEngine) -> None:
    # Valid positions without move tail must be silent.
    await eng.send("position startpos")
    got = await eng.read_silence(0.2)
    check(not got, f"position startpos should be silent: {got}")

    start_fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
    await eng.send(f"position fen {start_fen}")
    got = await eng.read_silence(0.2)
    check(not got, f"valid fen without moves should be silent: {got}")

    # With a legal move tail the engine applies the moves silently.
    await eng.send("position startpos moves e2e4 e7e5")
    got = await eng.read_silence(0.2)
    check(not got, f"legal moves tail should be silent: {got}")
    # Verify via d that the position was updated (pawn on e4)
    await eng.send("d")
    lines = await eng.read_until(lambda l: l.startswith("Fen:"), timeout=1.0)
    fen_line = next((l for l in lines if l.startswith("Fen:")), "")
    check("4P3" in fen_line or "e4" in fen_line or "rnbqkbnr/pppp1ppp/8/4p3/4P3" in fen_line, f"d after moves should show e4 pawn: {fen_line}")
    # Drain Checkers line
    await eng.read_until(lambda l: l.startswith("Checkers:"), timeout=1.0)

    # Invalid FEN must not crash — engine replies with `info string` and stays alive.
    await eng.send("position fen 8/8/8/8/8/8/8/8 w - - 0 1 moves a1a2")
    got = await eng.read_silence(0.2)
    check(any("error" in l.lower() for l in got), f"invalid fen should give error: {got}")
    lines = await eng.isready()
    check(lines[-1] == "readyok", f"engine unresponsive after invalid fen: {lines}")


async def test_ucinewgame(eng: UciEngine) -> None:
    await eng.send("ucinewgame")
    got = await eng.read_silence(0.2)
    check(not got, f"ucinewgame must be silent: {got}")
    lines = await eng.isready()
    check(lines[-1] == "readyok", f"engine unresponsive after ucinewgame: {lines}")


async def test_quit(eng: UciEngine) -> None:
    await eng.send("quit")
    await asyncio.wait_for(eng.proc.wait(), 5.0)  # raises on hang


TESTS = [
    test_no_startup_output,
    test_uci_handshake,
    test_isready,
    test_debug,
    test_register,
    test_unknown_commands,
    test_setoption,
    test_position_accepted,
    test_ucinewgame,
    test_quit,
]


async def main() -> None:
    if not BITWARK_BIN.exists():
        raise SystemExit(f"engine binary not found: {BITWARK_BIN}\nrun: cargo build --release")
    eng = UciEngine(BITWARK_BIN)
    await eng.start()
    try:
        for test in TESTS:
            try:
                await test(eng)
            except Exception as exc:
                print(f"FAIL  {test.__name__}: {exc}")
                if eng.stderr_lines:
                    print(f"      engine stderr (last 10): {eng.stderr_lines[-10:]}")
                raise SystemExit(1)
            print(f"PASS  {test.__name__}")
    finally:
        await eng.close()
    print(f"\n{len(TESTS)}/{len(TESTS)} passed — UCI gate GREEN")


if __name__ == "__main__":
    asyncio.run(main())

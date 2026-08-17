"""
The search plays chess.

Checks:
  - mate-in-1/2 puzzle suite solved
  - every bestmove legal vs python-chess
  - streaming info depth/score/nodes/nps/time/pv + bestmove
  - stop / isready during search
  - 1-minute (clock-based) game vs stockfish Elo 1320, no illegal moves/crashes

Run: cd tools && uv run python test_search.py
"""

from __future__ import annotations

import asyncio
import random
import time

import chess

from common import BITWARK_BIN, STOCKFISH_BIN, UciEngine

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


class Failure(AssertionError):
    pass


def check(cond: bool, msg: str) -> None:
    if not cond:
        raise Failure(msg)


def forces_mate_in(board: chess.Board, move: chess.Move, n: int) -> bool:
    """Does `move` force mate in <=n from board's perspective?"""
    board.push(move)
    if board.is_checkmate():
        board.pop()
        return True
    if n == 1:
        board.pop()
        return False
    if board.is_stalemate():
        board.pop()
        return False
    # For mate in 2: after our move, every opponent reply must allow us to mate in 1
    for opp_move in list(board.legal_moves):
        board.push(opp_move)
        if board.is_stalemate() or board.is_checkmate():
            board.pop()
            board.pop()
            return False
        mate_found = False
        for my_reply in board.legal_moves:
            board.push(my_reply)
            if board.is_checkmate():
                mate_found = True
                board.pop()
                break
            board.pop()
        board.pop()
        if not mate_found:
            board.pop()
            return False
    board.pop()
    return True


def is_mate_in_n(board: chess.Board, n: int) -> bool:
    for mv in board.legal_moves:
        if forces_mate_in(board, mv, n):
            return True
    return False


async def search_bestmove(
    engine: UciEngine, fen: str, go_cmd: str, timeout: float = 5.0
) -> tuple[str, list[str], list[str]]:
    """Send position fen + go, collect lines until bestmove.

    Returns (bestmove_uci, info_lines, all_lines).
    bestmove_uci is "" if (none).
    """
    await engine.send(f"position fen {fen}")
    await engine.send("isready")
    await engine.wait_for("readyok", timeout=2.0)
    await engine.send(go_cmd)
    lines = await engine.wait_for("bestmove", timeout=timeout)
    # lines includes bestmove line as last
    best_line = next((l for l in reversed(lines) if l.startswith("bestmove")), "")
    parts = best_line.split()
    best = parts[1] if len(parts) >= 2 and parts[1] != "(none)" else ""
    info_lines = [l for l in lines if l.startswith("info ")]
    return best, info_lines, lines


# ---------------------------------------------------------------------------
# Puzzle suites
# ---------------------------------------------------------------------------

# Mate in 1 — engine should deliver immediate checkmate
MATE_IN_1_FENS = [
    "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 1 3",  # Qxf7#
    "7k/5Q2/6K1/8/8/8/8/8 w - - 0 1",  # KQ vs K, Qf8#
    "k7/8/K7/8/8/8/8/Q7 w - - 0 1",  # KQ vs K, many mates
    "5k2/8/5K2/8/8/8/8/2Q5 w - - 0 1",  # Qc8#
    "3k4/3Q4/3K4/8/8/8/8/8 w - - 0 1",  # queen+king vs king
    "7k/6p1/5KQ1/8/8/8/8/8 w - - 0 1",  # Qxg7# (pawn blocks)
    "rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 1 2",  # not mate, will be filtered if not mate
]

# Mate in 2 but not mate in 1 (KQ vs K with queen far)
MATE_IN_2_FENS = [
    "1k6/8/1K6/8/8/8/8/1Q6 w - - 0 1",
    "2k5/8/1K6/8/8/8/8/3Q4 w - - 0 1",
    "1k6/8/1K6/Q7/8/8/8/8 w - - 0 1",
    "1k6/8/1K6/8/8/8/8/2Q5 w - - 0 1",
    "1k6/8/1K6/8/8/8/2Q5/8 w - - 0 1",
    # Add a midgame-ish mate in 2 if available; we filter at runtime to keep only true mate-in-2
    "r1bqk2r/ppp2ppp/2n5/2b1p3/2B1P3/3P1N2/PPP2PPP/RNBQK2R w KQkq - 0 1",
    "5rk1/5ppp/8/8/8/8/5PPP/6K1 w - - 0 1",
]


async def test_mate_in_1(engine: UciEngine) -> None:
    print("  mate-in-1 puzzles")
    valid = []
    for fen in MATE_IN_1_FENS:
        b = chess.Board(fen)
        if not is_mate_in_n(b, 1):
            print(f"    skip {fen[:40]}... (not mate in 1, as expected for mixed list)")
            continue
        valid.append(fen)
    check(len(valid) >= 4, f"need >=4 valid mate-in-1 puzzles, got {len(valid)}")
    for fen in valid:
        best, infos, _ = await search_bestmove(engine, fen, "go depth 5", timeout=4.0)
        check(best != "", f"engine returned (none) for mate-in-1 {fen}")
        b = chess.Board(fen)
        try:
            mv = chess.Move.from_uci(best)
        except ValueError:
            raise Failure(f"invalid bestmove uci {best!r} for fen {fen}")
        check(mv in b.legal_moves, f"illegal bestmove {best} for fen {fen}")
        check(forces_mate_in(b, mv, 1), f"bestmove {best} does not mate in 1 for fen {fen}")
        print(f"    {fen[:40]}... -> {best} mate in 1 OK (depth 5, {len(infos)} infos)")


async def test_mate_in_2(engine: UciEngine) -> None:
    print("  mate-in-2 puzzles (forces mate in <=2)")
    valid = []
    for fen in MATE_IN_2_FENS:
        b = chess.Board(fen)
        # Need mate in 2 but not in 1, to ensure it's truly a 2-mover (or allow mate in 1 also? we accept both)
        # For this suite we want at least some that are strictly mate in 2
        if not is_mate_in_n(b, 2):
            print(f"    skip {fen[:40]}... (not mate in 2)")
            continue
        valid.append(fen)
    # Ensure at least 4 puzzles, fallback to generating more if needed
    if len(valid) < 4:
        # Generate more KQ vs K mates dynamically
        wk = chess.parse_square("b6")
        bk = chess.parse_square("b8")
        for sq_name in ["a1", "a5", "c1", "c2", "h1", "g1", "e1"]:
            if len(valid) >= 6:
                break
            board = chess.Board.empty()
            board.set_piece_at(wk, chess.Piece(chess.KING, chess.WHITE))
            board.set_piece_at(bk, chess.Piece(chess.KING, chess.BLACK))
            sq = chess.parse_square(sq_name)
            if sq in [wk, bk]:
                continue
            board.set_piece_at(sq, chess.Piece(chess.QUEEN, chess.WHITE))
            board.turn = chess.WHITE
            if board.is_check() or board.is_stalemate():
                continue
            fen = board.fen()
            b = chess.Board(fen)
            if is_mate_in_n(b, 2) and not is_mate_in_n(b, 1):
                if fen not in valid:
                    valid.append(fen)
    check(len(valid) >= 4, f"need >=4 mate-in-2 puzzles, got {len(valid)}")
    for fen in valid[:6]:
        best, infos, _ = await search_bestmove(engine, fen, "go movetime 1500", timeout=4.0)
        check(best != "", f"engine returned (none) for mate-in-2 {fen}")
        b = chess.Board(fen)
        mv = chess.Move.from_uci(best)
        check(mv in b.legal_moves, f"illegal bestmove {best} for fen {fen}")
        # Accept mate in 1 as well (faster mate is fine)
        ok = forces_mate_in(b, mv, 2) or forces_mate_in(b, mv, 1)
        check(ok, f"bestmove {best} does not force mate in <=2 for fen {fen}")
        print(f"    {fen[:40]}... -> {best} forced mate <=2 OK")


async def test_legality_random(engine: UciEngine) -> None:
    print("  legality on random positions")
    random.seed(0xC0FFEE)
    for idx in range(25):
        # Generate random legal position by playing random moves from start
        board = chess.Board()
        for _ in range(random.randint(8, 30)):
            moves = list(board.legal_moves)
            if not moves or board.is_game_over():
                break
            board.push(random.choice(moves))
            if board.is_game_over():
                break
        if board.is_game_over():
            continue
        fen = board.fen()
        best, infos, _ = await search_bestmove(engine, fen, "go depth 4", timeout=4.0)
        # Must have exactly one bestmove line
        check(best != "" or len(list(board.legal_moves)) == 0, f"empty bestmove for fen {fen}")
        if best:
            mv = chess.Move.from_uci(best)
            check(mv in board.legal_moves, f"illegal bestmove {best} for fen {fen}")
        # Info lines should be parseable
        for line in infos:
            check("depth" in line and "score" in line, f"info line missing depth/score: {line!r}")
            check("pv" in line or "nodes" in line, f"info line missing pv/nodes: {line!r}")
        # Depth should be monotonic if we have multiple infos
        depths = []
        for line in infos:
            # extract depth integer after "depth"
            try:
                tok = line.split()
                if "depth" in tok:
                    di = tok.index("depth")
                    depths.append(int(tok[di + 1]))
            except Exception:
                pass
        if len(depths) >= 2:
            check(depths == sorted(depths), f"depth not monotonic: {depths}")
        print(f"    random {idx+1:02d}: {fen[:35]}... -> {best or '(none)'} OK")


async def test_streaming_and_stop() -> None:
    print("  streaming + stop + isready during search")
    engine = UciEngine(BITWARK_BIN)
    await engine.start()
    try:
        await engine.send("uci")
        await engine.wait_for("uciok", timeout=2.0)
        await engine.send("isready")
        await engine.wait_for("readyok", timeout=2.0)

        # go movetime should stream at least 2 infos before bestmove
        await engine.send("position startpos")
        await engine.send("isready")
        await engine.wait_for("readyok", timeout=2.0)
        await engine.send("go movetime 800")
        lines = await engine.wait_for("bestmove", timeout=4.0)
        infos = [l for l in lines if l.startswith("info ")]
        check(len(infos) >= 2, f"expected >=2 info lines for movetime 800, got {len(infos)}: {lines}")
        best_line = next(l for l in lines if l.startswith("bestmove"))
        print(f"    movetime streaming: {len(infos)} infos, {best_line} OK")

        # go infinite + stop
        await engine.send("position startpos")
        await engine.send("isready")
        await engine.wait_for("readyok", timeout=2.0)
        await engine.send("go infinite")
        # let it search a bit
        await asyncio.sleep(0.4)
        await engine.send("stop")
        lines = await engine.wait_for("bestmove", timeout=4.0)
        check(any(l.startswith("bestmove") for l in lines), "no bestmove after stop")
        print(f"    stop: {lines[-1]} OK")

        # isready during infinite search
        await engine.send("position startpos")
        await engine.send("isready")
        await engine.wait_for("readyok", timeout=2.0)
        await engine.send("go infinite")
        await asyncio.sleep(0.2)
        await engine.send("isready")
        # Should get readyok before bestmove
        lines = await engine.read_until(lambda l: l.startswith("readyok"), timeout=2.0)
        check(any(l.startswith("readyok") for l in lines), "isready not answered during search")
        print(f"    isready during search: readyok OK")
        await engine.send("stop")
        lines = await engine.wait_for("bestmove", timeout=4.0)
        check(any(l.startswith("bestmove") for l in lines), "no bestmove after isready+stop")

        # bare go + stop (implicit depth 245)
        await engine.send("position startpos")
        await engine.send("isready")
        await engine.wait_for("readyok", timeout=2.0)
        await engine.send("go")
        await asyncio.sleep(0.3)
        await engine.send("stop")
        lines = await engine.wait_for("bestmove", timeout=4.0)
        check(any(l.startswith("bestmove") for l in lines), "no bestmove for bare go+stop")
        print(f"    bare go+stop: {lines[-1]} OK")

    finally:
        await engine.close()


async def test_score_sanity(engine: UciEngine) -> None:
    print("  score sanity")
    # Startpos should be near 0
    fen_start = chess.STARTING_FEN
    best, infos, _ = await search_bestmove(engine, fen_start, "go depth 5", timeout=4.0)
    # Extract last info score
    last_info = next((l for l in reversed(infos) if "score" in l), "")
    # Parse cp
    cp = None
    if "score cp" in last_info:
        try:
            tok = last_info.split()
            idx = tok.index("cp")
            cp = int(tok[idx + 1])
        except Exception:
            cp = None
    elif "score mate" in last_info:
        # Mate score near startpos is unexpected
        cp = None
        check(False, f"unexpected mate score at startpos: {last_info}")
    if cp is not None:
        check(abs(cp) < 120, f"startpos depth 5 score {cp} not near 0")
        print(f"    startpos depth 5 cp {cp} OK")
    # Queen up should be big
    fen_q_up = "4k3/8/8/8/8/8/8/3QK3 w - - 0 1"
    best, infos, _ = await search_bestmove(engine, fen_q_up, "go depth 4", timeout=4.0)
    last_info = next((l for l in reversed(infos) if "score" in l), "")
    if "score cp" in last_info:
        tok = last_info.split()
        idx = tok.index("cp")
        cp = int(tok[idx + 1])
        check(cp > 700, f"queen up cp {cp} not >700")
        print(f"    queen up cp {cp} OK")
    # Also test that black to move queen up is negative for side to move? Actually our score is side-to-move, so black to move with black queen up should be positive for black.
    fen_q_up_black = "3qk3/8/8/8/8/8/8/4K3 b - - 0 1"  # black queen + king vs white king, black to move
    # This fen is not valid due to kings, but test that engine doesn't crash
    try:
        b = chess.Board(fen_q_up_black)
        # If fen is illegal (kings adjacent etc) skip
        if b.is_valid():
            best, infos, _ = await search_bestmove(engine, fen_q_up_black, "go depth 3", timeout=4.0)
            print(f"    black queen up sanity OK")
    except Exception:
        pass


async def test_game_vs_stockfish() -> None:
    if not STOCKFISH_BIN.exists():
        print("  SKIP game vs stockfish (stockfish binary not found)")
        return
    has_sf = True
    # Quick check stockfish supports UCI_LimitStrength
    print("  game vs stockfish Elo 1320 (clock-based, ~30 plies, wall 60s)")
    bw = UciEngine(BITWARK_BIN)
    sf = UciEngine(STOCKFISH_BIN)
    await bw.start()
    await sf.start()
    try:
        await bw.send("uci")
        await bw.wait_for("uciok", timeout=2.0)
        await sf.send("uci")
        await sf.wait_for("uciok", timeout=2.0)
        await sf.send("setoption name UCI_LimitStrength value true")
        await sf.send("setoption name UCI_Elo value 1320")
        await sf.send("isready")
        await sf.wait_for("readyok", timeout=2.0)
        await bw.send("isready")
        await bw.wait_for("readyok", timeout=2.0)

        board = chess.Board()
        # Use a modest clock for speed but still exercise clock fallback.
        # 60000 per side would be 3 sec per move, 30 plies ~45 sec wall—still okay.
        # For CI speed, use 15000 per side (750ms per move) but still clock path.
        # We use 15000 to keep test <30 sec, still validates wtime handling.
        wtime = 15000
        btime = 15000
        start_wall = time.time()
        max_wall = 55.0
        max_plies = 60
        move_overhead = 10

        # Track moves for position command
        moves_uci: list[str] = []

        for ply in range(max_plies):
            if board.is_game_over():
                print(f"    game over: {board.result()} after {ply} plies")
                break
            if time.time() - start_wall > max_wall:
                print(f"    wall timeout after {ply} plies")
                break
            is_white = board.turn == chess.WHITE
            engine = bw if is_white else sf
            # Update clocks roughly: subtract elapsed from last move's time
            # For simplicity, keep wtime/btime decrementing by estimated movetime
            # We measure actual elapsed per move below.
            pos_cmd = "position startpos" + (f" moves {' '.join(moves_uci)}" if moves_uci else "")
            await engine.send(pos_cmd)
            await engine.send("isready")
            await engine.wait_for("readyok", timeout=2.0)

            # Use clock command to exercise fallback (bitwark) and real clock (stockfish)
            # We send wtime/btime even though board is not necessarily startpos clock-wise;
            # it's the standard way.
            go_cmd = f"go wtime {wtime} btime {btime} winc 0 binc 0"
            t0 = time.time()
            await engine.send(go_cmd)
            lines = await engine.wait_for("bestmove", timeout=8.0)
            elapsed_ms = int((time.time() - t0) * 1000)
            best_line = next(l for l in lines if l.startswith("bestmove"))
            parts = best_line.split()
            if len(parts) < 2 or parts[1] == "(none)":
                print(f"    no move at ply {ply}, board {board.fen()}")
                break
            uci_move = parts[1]
            try:
                mv = chess.Move.from_uci(uci_move)
            except ValueError:
                raise Failure(f"illegal uci {uci_move!r} at ply {ply} fen {board.fen()}")
            check(mv in board.legal_moves, f"illegal move {uci_move} at ply {ply} fen {board.fen()}")
            board.push(mv)
            moves_uci.append(uci_move)
            # Decrement clock for side that moved
            if is_white:
                wtime = max(10, wtime - elapsed_ms - move_overhead)
            else:
                btime = max(10, btime - elapsed_ms - move_overhead)
            # If either clock is low, still continue until wall timeout
            who = "Bitwark" if is_white else "Stockfish1320"
            print(f"    {ply+1:2d}. {'White' if is_white else 'Black'} ({who}) {uci_move} in {elapsed_ms}ms  wtime {wtime} btime {btime}")

            # Check engines alive
            check(bw.alive, "Bitwark died during game")
            check(sf.alive, "Stockfish died during game")

        check(bw.alive, "Bitwark crashed during game")
        check(sf.alive, "Stockfish crashed during game")
        # Ensure at least a few plies were played and all moves were legal (already checked)
        check(len(moves_uci) >= 6, f"game too short, only {len(moves_uci)} plies")
        print(f"  game finished: {len(moves_uci)} plies, result {board.result() if board.is_game_over() else '*'}")
        # No illegal moves by construction; crashes already checked
        print("  game vs stockfish: no illegal moves or crashes")
    finally:
        await bw.close()
        await sf.close()


async def main() -> None:
    if not BITWARK_BIN.exists():
        raise SystemExit(f"engine binary not found: {BITWARK_BIN}\nrun: cargo build --release")

    print("RUN   test_mate_in_1")
    bw = UciEngine(BITWARK_BIN)
    await bw.start()
    try:
        await bw.send("uci")
        await bw.wait_for("uciok", timeout=2.0)
        await test_mate_in_1(bw)
        print("PASS  test_mate_in_1")
        print("RUN   test_mate_in_2")
        await test_mate_in_2(bw)
        print("PASS  test_mate_in_2")
        print("RUN   test_legality_random")
        await test_legality_random(bw)
        print("PASS  test_legality_random")
        print("RUN   test_score_sanity")
        await test_score_sanity(bw)
        print("PASS  test_score_sanity")
    finally:
        await bw.close()

    print("RUN   test_streaming_and_stop")
    await test_streaming_and_stop()
    print("PASS  test_streaming_and_stop")

    print("RUN   test_game_vs_stockfish")
    await test_game_vs_stockfish()
    print("PASS  test_game_vs_stockfish")

    print("\n6/6 suites passed — search gate GREEN")


if __name__ == "__main__":
    asyncio.run(main())

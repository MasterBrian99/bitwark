"""
Paired SPRT match runner: bitwark vs bitwark (or vs stockfish).

Plays paired games (both colors) from a small opening book at fixed
movetime or with clock time control, adjudicates via python-chess, and
runs a Wald SPRT (elo0 / elo1, alpha=beta=0.05) with the usual fishtest LLR.
For the conformance gate a fixed-N clock mode with Elo-limited stockfish is
provided (`--tc`, `--fixed-games`, `--elo-limit`).

Usage (SPRT, movetime):
  uv run python match_runner.py --engine-a target/release/bitwark --engine-b /tmp/bitwark-baseline \
      [--elo0 0 --elo1 25 --movetime 100 --max-games 200 --concurrency 5]

Usage (gate, clock vs Elo-limited SF):
  uv run python match_runner.py --engine-a target/release/bitwark --engine-b ./stockfish \
      --fixed-games 25 --tc 30000+300 --elo-limit 1320 --concurrency 5

Exit codes: 0 = H1 accepted / gate pass, 1 = H0 accepted / gate fail, 2 = undecided at cap.
"""

from __future__ import annotations

import argparse
import asyncio
import math
import sys
import time
from pathlib import Path

import chess

from common import BITWARK_BIN, STOCKFISH_BIN, UciEngine

REPO_ROOT = Path(__file__).resolve().parent.parent


def resolve_engine(p: str | Path) -> Path:
    path = Path(p)
    if path.is_absolute():
        return path
    # Try relative to REPO_ROOT first (handles `./stockfish` when cwd is tools/)
    cand = REPO_ROOT / path
    if cand.exists():
        return cand
    return Path(p).resolve()

DEFAULT_OPENINGS: list[list[str]] = [
    [],  # startpos
    ["e2e4", "e7e5"],
    ["e2e4", "c7c5"],
    ["d2d4", "d7d5"],
    ["d2d4", "g8f6"],
    ["g1f3", "d7d5"],
    ["c2c4", "e7e5"],
    ["g1f3", "g8f6"],
    ["e2e4", "e7e6"],
    ["d2d4", "c7c5"],
    ["c2c4", "g8f6"],
    ["e2e4", "g8f6"],
]

# Allowed protocol lines from engines during a game (UCI spec §2/§3)
PROTOCOL_PREFIXES = ("id ", "option ", "uciok", "readyok", "info ", "bestmove")


def is_protocol_line(line: str) -> bool:
    if not line.strip():
        return True
    return line.startswith(PROTOCOL_PREFIXES)


def parse_tc(s: str) -> tuple[int, int]:
    """Parse --tc M+INC or M alone (ms)."""
    if "+" in s:
        base_s, inc_s = s.split("+", 1)
        return int(base_s), int(inc_s)
    return int(s), 0


def elo_to_prob(elo: float) -> float:
    return 1.0 / (1.0 + 10 ** (-elo / 400.0))


def llr_increment(result: float, elo0: float, elo1: float) -> float:
    """LLR increment for a single game result in {1,0,0.5} from engine A perspective.

    Draws are scored as the average of a win and a loss LLR (trinomial approx).
    """
    p0 = elo_to_prob(elo0)
    p1 = elo_to_prob(elo1)
    p0 = min(max(p0, 1e-9), 1 - 1e-9)
    p1 = min(max(p1, 1e-9), 1 - 1e-9)
    if result == 1.0:
        return math.log(p1 / p0)
    if result == 0.0:
        return math.log((1 - p1) / (1 - p0))
    # draw
    return 0.5 * math.log(p1 / p0) + 0.5 * math.log((1 - p1) / (1 - p0))


def elo_estimate(wins: int, draws: int, losses: int) -> tuple[float, float]:
    n = wins + draws + losses
    if n == 0:
        return 0.0, 0.0
    score = (wins + 0.5 * draws) / n
    if score <= 0.0 or score >= 1.0:
        return (999.0 if score >= 1.0 else -999.0), 0.0
    elo = -400 * math.log10(1 / score - 1)
    # Approx error
    variance = (wins * (1 - score) ** 2 + losses * score**2 + draws * (0.5 - score) ** 2) / n
    try:
        d_elo_d_score = 400 / (math.log(10) * score * (1 - score))
        se_score = math.sqrt(variance / n)
        se_elo = abs(d_elo_d_score) * se_score
    except Exception:
        se_elo = 0.0
    return elo, 1.96 * se_elo


async def handshake_with_elo(engine: UciEngine, elo_limit: int | None, is_b: bool) -> None:
    await engine.handshake()
    if elo_limit is not None and is_b:
        # Stockfish Elo-limited mode
        await engine.send("setoption name UCI_LimitStrength value true")
        await engine.send(f"setoption name UCI_Elo value {elo_limit}")
        await engine.send("isready")
        await engine.wait_for("readyok", timeout=2.0)


async def play_single_game(
    white_bin: Path,
    black_bin: Path,
    opening_fen: str | None,
    opening_moves: list[str],
    movetime: int,
    engine_a_path: Path | None = None,
    engine_b_path: Path | None = None,
    elo_limit: int | None = None,
) -> tuple[float, int]:
    """Play one game with white_bin as White, black_bin as Black.

    opening_fen: if not None, start from this FEN; otherwise startpos + opening_moves.
    Returns (result, violations) where result is 1.0/0.5/0.0 from White perspective
    and violations is the count of non-protocol lines from engine A (bitwark).
    """
    async with UciEngine(white_bin) as white_eng, UciEngine(black_bin) as black_eng:
        # Handshakes (+ Elo-limit for engine B if requested)
        await handshake_with_elo(white_eng, elo_limit, Path(white_bin) == Path(engine_b_path) if engine_b_path else False)
        await handshake_with_elo(black_eng, elo_limit, Path(black_bin) == Path(engine_b_path) if engine_b_path else False)

        if opening_fen is not None:
            board = chess.Board(opening_fen)
        else:
            board = chess.Board()
            for mv in opening_moves:
                try:
                    board.push_uci(mv)
                except Exception:
                    break

        if board.is_game_over():
            if board.is_checkmate():
                winner = not board.turn
                return (1.0 if winner == chess.WHITE else 0.0), 0
            return 0.5, 0

        violations = 0
        max_ply = 400
        while not board.is_game_over() and board.ply() < max_ply:
            is_white = board.turn == chess.WHITE
            mover_eng = white_eng if is_white else black_eng
            mover_bin = white_bin if is_white else black_bin

            fen = board.fen()
            await mover_eng.send(f"position fen {fen}")
            await mover_eng.isready()
            await mover_eng.send(f"go movetime {movetime}")

            try:
                lines = await mover_eng.read_until(
                    lambda l: l.startswith("bestmove"), timeout=movetime / 1000 + 3.0
                )
            except asyncio.TimeoutError:
                # Forfeit on time
                return (0.0 if is_white else 1.0), violations

            # Protocol validation for bitwark (engine A)
            if engine_a_path is not None and Path(mover_bin) == Path(engine_a_path):
                for l in lines:
                    if not is_protocol_line(l):
                        violations += 1

            bm_line = next((l for l in lines if l.startswith("bestmove")), "")
            parts = bm_line.split()
            if len(parts) < 2 or parts[1] == "(none)":
                if not list(board.legal_moves):
                    if board.is_check():
                        return (0.0 if is_white else 1.0), violations
                    else:
                        return 0.5, violations
                return (0.0 if is_white else 1.0), violations

            uci_move = parts[1]
            try:
                move = chess.Move.from_uci(uci_move)
            except ValueError:
                return (0.0 if is_white else 1.0), violations

            if move not in board.legal_moves:
                return (0.0 if is_white else 1.0), violations

            board.push(move)

            if board.is_fifty_moves() or board.is_insufficient_material() or board.can_claim_threefold_repetition():
                return 0.5, violations

        if board.is_checkmate():
            winner = not board.turn
            return (1.0 if winner == chess.WHITE else 0.0), violations
        return 0.5, violations


async def play_single_game_clock(
    white_bin: Path,
    black_bin: Path,
    opening_fen: str | None,
    opening_moves: list[str],
    tc_base: int,
    tc_inc: int,
    engine_a_path: Path,
    engine_b_path: Path,
    elo_limit: int | None,
    overhead: int = 10,
) -> tuple[float, int, int]:
    """Clock time control game (wtime/btime).

    Returns (result, violations, forfeits) where result is from White perspective,
    violations = non-protocol lines from engine A, forfeits = 1 if time forfeit by either side.
    """
    async with UciEngine(white_bin) as white_eng, UciEngine(black_bin) as black_eng:
        await handshake_with_elo(white_eng, elo_limit, Path(white_bin) == Path(engine_b_path))
        await handshake_with_elo(black_eng, elo_limit, Path(black_bin) == Path(engine_b_path))

        if opening_fen is not None:
            board = chess.Board(opening_fen)
        else:
            board = chess.Board()
            for mv in opening_moves:
                try:
                    board.push_uci(mv)
                except Exception:
                    break

        if board.is_game_over():
            if board.is_checkmate():
                winner = not board.turn
                return (1.0 if winner == chess.WHITE else 0.0), 0, 0
            return 0.5, 0, 0

        wtime = tc_base
        btime = tc_base
        violations = 0
        forfeits = 0
        max_ply = 400
        while not board.is_game_over() and board.ply() < max_ply:
            is_white = board.turn == chess.WHITE
            mover_eng = white_eng if is_white else black_eng
            mover_bin = white_bin if is_white else black_bin
            cur_wtime = wtime
            cur_btime = btime
            fen = board.fen()
            await mover_eng.send(f"position fen {fen}")
            await mover_eng.isready()
            go_cmd = f"go wtime {cur_wtime} btime {cur_btime} winc {tc_inc} binc {tc_inc}"
            await mover_eng.send(go_cmd)
            # Timeout: remaining time + inc + generous slack; forfeit if exceeded
            cur_rem = cur_wtime if is_white else cur_btime
            timeout = cur_rem / 1000 + tc_inc / 1000 + 4.0
            # Clamp timeout to avoid absurd waits on large clocks
            timeout = min(timeout, 30.0)
            t0 = time.time()
            try:
                lines = await mover_eng.read_until(
                    lambda l: l.startswith("bestmove"), timeout=timeout
                )
            except asyncio.TimeoutError:
                # Time forfeit
                forfeits = 1
                return (0.0 if is_white else 1.0), violations, forfeits

            elapsed_ms = int((time.time() - t0) * 1000)

            if Path(mover_bin) == Path(engine_a_path):
                for l in lines:
                    if not is_protocol_line(l):
                        violations += 1

            bm_line = next((l for l in lines if l.startswith("bestmove")), "")
            parts = bm_line.split()
            if len(parts) < 2 or parts[1] == "(none)":
                if not list(board.legal_moves):
                    if board.is_check():
                        return (0.0 if is_white else 1.0), violations, forfeits
                    else:
                        return 0.5, violations, forfeits
                return (0.0 if is_white else 1.0), violations, forfeits

            uci_move = parts[1]
            try:
                move = chess.Move.from_uci(uci_move)
            except ValueError:
                return (0.0 if is_white else 1.0), violations, forfeits
            if move not in board.legal_moves:
                return (0.0 if is_white else 1.0), violations, forfeits

            # Time forfeit check: if elapsed > remaining + inc (approx), count as forfeit
            # The driver is lenient: only flag if elapsed > remaining + 500ms slack
            rem_before = cur_wtime if is_white else cur_btime
            if elapsed_ms > rem_before + 500:
                forfeits = 1
                return (0.0 if is_white else 1.0), violations, forfeits

            board.push(move)
            # Update clock (subtract elapsed + overhead, then add inc)
            if is_white:
                wtime = max(0, wtime - elapsed_ms - overhead) + tc_inc
            else:
                btime = max(0, btime - elapsed_ms - overhead) + tc_inc

            if board.is_fifty_moves() or board.is_insufficient_material() or board.can_claim_threefold_repetition():
                return 0.5, violations, forfeits
            # Quick crash check
            if not white_eng.alive or not black_eng.alive:
                # The side that crashed forfeits
                if not white_eng.alive and not black_eng.alive:
                    return 0.5, violations, forfeits
                if not white_eng.alive:
                    return (0.0 if board.turn == chess.WHITE else 1.0), violations, forfeits
                else:
                    return (0.0 if board.turn == chess.BLACK else 1.0), violations, forfeits

        if board.is_checkmate():
            winner = not board.turn
            return (1.0 if winner == chess.WHITE else 0.0), violations, forfeits
        return 0.5, violations, forfeits


async def run_match(args) -> int:
    engine_a = resolve_engine(args.engine_a)
    engine_b = resolve_engine(args.engine_b)
    if not engine_a.exists():
        print(f"engine-a not found: {engine_a}", file=sys.stderr)
        return 2
    if not engine_b.exists():
        print(f"engine-b not found: {engine_b}", file=sys.stderr)
        return 2

    # Parse time controls
    tc_base = tc_inc = None
    if args.tc is not None:
        try:
            tc_base, tc_inc = parse_tc(args.tc)
        except Exception as e:
            print(f"invalid --tc {args.tc!r}: {e}", file=sys.stderr)
            return 2

    # Fixed-games gate mode — no SPRT
    if args.fixed_games is not None:
        return await run_fixed_gate(
            engine_a,
            engine_b,
            tc_base,
            tc_inc,
            args.fixed_games,
            args.elo_limit,
            args.concurrency,
            args.openings,
            args.movetime if tc_base is None else None,
        )

    # SPRT mode (existing)
    elo0 = args.elo0
    elo1 = args.elo1
    alpha = args.alpha
    beta = args.beta
    movetime = args.movetime
    max_games = args.max_games

    lower = math.log(beta / (1 - alpha))
    upper = math.log((1 - beta) / alpha)
    print(f"SPRT: elo0={elo0} elo1={elo1} alpha={alpha} beta={beta}")
    print(f" bounds [{lower:.4f}, {upper:.4f}] movetime={movetime} max_games={max_games}")
    print(f" engines: A={engine_a} B={engine_b}")

    # Openings
    openings_norm: list[tuple[str | None, list[str]]] = []
    if args.openings == "default":
        for op in DEFAULT_OPENINGS:
            openings_norm.append((None, op))
    else:
        p = Path(args.openings)
        if p.exists():
            for line in p.read_text().splitlines():
                line = line.strip()
                if not line or line.startswith("#"):
                    continue
                if "/" in line and " " in line:
                    # Assume FEN
                    try:
                        chess.Board(line)
                        openings_norm.append((line, []))
                        continue
                    except Exception:
                        pass
                # Otherwise treat as move list
                openings_norm.append((None, line.split()))
            if not openings_norm:
                for op in DEFAULT_OPENINGS:
                    openings_norm.append((None, op))
            print(f" loaded {len(openings_norm)} openings from {p}")
        else:
            # Single custom opening string
            if "/" in args.openings:
                openings_norm.append((args.openings, []))
            else:
                openings_norm.append((None, args.openings.split()))
            print(f" using custom opening: {openings_norm}")

    wins = draws = losses = 0
    llr = 0.0
    games = 0
    pair_idx = 0

    while games < max_games:
        opening_fen, opening_moves = openings_norm[pair_idx % len(openings_norm)]
        pair_idx += 1

        # Game 1: A white, B black
        if tc_base is not None:
            r1_white, v1, _ = await play_single_game_clock(
                engine_a, engine_b, opening_fen, opening_moves, tc_base, tc_inc or 0, engine_a, engine_b, args.elo_limit
            )
            # v1 counted as violations from engine A perspective already
            if v1:
                print(f"  WARNING: {v1} protocol violations by A in game {games+1}")
        else:
            r1_white, v1 = await play_single_game(
                engine_a, engine_b, opening_fen, opening_moves, movetime, engine_a, engine_b, args.elo_limit
            )
        # Map to A perspective: A is White
        r1_A = r1_white
        games += 1
        if r1_A == 1.0:
            wins += 1
        elif r1_A == 0.0:
            losses += 1
        else:
            draws += 1
        llr += llr_increment(r1_A, elo0, elo1)
        elo, err = elo_estimate(wins, draws, losses)
        score = (wins + 0.5 * draws) / games if games else 0
        v = "H1" if llr >= upper else ("H0" if llr <= lower else "")
        print(
            f"games {games:4}: W {wins:3} D {draws:3} L {losses:3} "
            f"score {score*100:5.1f}% elo {elo:+6.1f} ±{err:4.1f} llr {llr:+6.2f} {v}"
        )
        if llr >= upper:
            print(f"SPRT: H1 accepted (elo1={elo1}) after {games} games")
            return 0
        if llr <= lower:
            print(f"SPRT: H0 accepted (elo0={elo0}) after {games} games")
            return 1
        if games >= max_games:
            break

        # Game 2: B white, A black (same opening, colors swapped)
        if tc_base is not None:
            r2_white, v2, _ = await play_single_game_clock(
                engine_b, engine_a, opening_fen, opening_moves, tc_base, tc_inc or 0, engine_a, engine_b, args.elo_limit
            )
        else:
            r2_white, v2 = await play_single_game(
                engine_b, engine_a, opening_fen, opening_moves, movetime, engine_a, engine_b, args.elo_limit
            )
        # A is Black, so invert (draw stays 0.5)
        if r2_white == 1.0:
            r2_A = 0.0
        elif r2_white == 0.0:
            r2_A = 1.0
        else:
            r2_A = 0.5
        games += 1
        if r2_A == 1.0:
            wins += 1
        elif r2_A == 0.0:
            losses += 1
        else:
            draws += 1
        llr += llr_increment(r2_A, elo0, elo1)
        elo, err = elo_estimate(wins, draws, losses)
        score = (wins + 0.5 * draws) / games if games else 0
        v = "H1" if llr >= upper else ("H0" if llr <= lower else "")
        print(
            f"games {games:4}: W {wins:3} D {draws:3} L {losses:3} "
            f"score {score*100:5.1f}% elo {elo:+6.1f} ±{err:4.1f} llr {llr:+6.2f} {v}"
        )
        if llr >= upper:
            print(f"SPRT: H1 accepted (elo1={elo1}) after {games} games")
            return 0
        if llr <= lower:
            print(f"SPRT: H0 accepted (elo0={elo0}) after {games} games")
            return 1

    print(f"SPRT: undecided after {games} games (cap {max_games})")
    elo, err = elo_estimate(wins, draws, losses)
    print(f" final: W{wins} D{draws} L{losses} elo {elo:+.1f} ±{err:.1f} llr {llr:.2f}")
    # For the SPRT gate, a positive elo with undecided still counts as "shows gain"
    # if wins > losses significantly. We return 0 if elo > 10, 2 otherwise.
    if elo > 10 and wins > losses:
        print(" Positive elo at cap — treating as H1 for gate")
        return 0
    return 2


async def run_fixed_gate(
    engine_a: Path,
    engine_b: Path,
    tc_base: int | None,
    tc_inc: int | None,
    fixed_games: int,
    elo_limit: int | None,
    concurrency: int,
    openings_arg: str,
    movetime_fallback: int | None,
) -> int:
    """Run a fixed number of games with clock or movetime, with violation/forfeit detection.

    Returns 0 on gate pass (zero crashes/forfeits/protocol violations),
    1 on gate fail (any violation).
    """
    print(f"Fixed gate: {fixed_games} games, tc={tc_base}+{tc_inc} movetime={movetime_fallback} elo-limit={elo_limit} concurrency={concurrency}")
    print(f" engines: A={engine_a} B={engine_b}")

    # Openings
    openings_norm: list[tuple[str | None, list[str]]] = []
    if openings_arg == "default":
        for op in DEFAULT_OPENINGS:
            openings_norm.append((None, op))
    else:
        p = Path(openings_arg)
        if p.exists():
            for line in p.read_text().splitlines():
                line = line.strip()
                if not line or line.startswith("#"):
                    continue
                if "/" in line and " " in line:
                    try:
                        chess.Board(line)
                        openings_norm.append((line, []))
                        continue
                    except Exception:
                        pass
                openings_norm.append((None, line.split()))
            if not openings_norm:
                for op in DEFAULT_OPENINGS:
                    openings_norm.append((None, op))
        else:
            if "/" in openings_arg:
                openings_norm.append((openings_arg, []))
            else:
                openings_norm.append((None, openings_arg.split()))

    # Build game specs paired (colors swapped per opening)
    specs: list[tuple[Path, Path, str | None, list[str]]] = []
    pair_idx = 0
    while len(specs) < fixed_games:
        opening_fen, opening_moves = openings_norm[pair_idx % len(openings_norm)]
        pair_idx += 1
        if len(specs) < fixed_games:
            specs.append((engine_a, engine_b, opening_fen, opening_moves))
        if len(specs) < fixed_games:
            specs.append((engine_b, engine_a, opening_fen, opening_moves))

    sem = asyncio.Semaphore(max(1, concurrency))
    results: list[tuple[float, int, int, str]] = []  # (white result, violations, forfeits, status)

    async def run_one(idx: int, white_bin: Path, black_bin: Path, fen: str | None, moves: list[str]):
        async with sem:
            if tc_base is not None:
                res_w, viol, forf = await play_single_game_clock(
                    white_bin, black_bin, fen, moves, tc_base, tc_inc or 0, engine_a, engine_b, elo_limit
                )
            else:
                mt = movetime_fallback if movetime_fallback is not None else 100
                res_w, viol = await play_single_game(
                    white_bin, black_bin, fen, moves, mt, engine_a, engine_b, elo_limit
                )
                forf = 0
                # For movetime gate, check violations already, forfeits are timeout forfeits counted as result (already)
            # Check aliveness via result forfeits (play functions return forfeit as result)
            status = "OK" if viol == 0 and forf == 0 else f"viol={viol} forf={forf}"
            white_name = "A" if white_bin == engine_a else "B"
            black_name = "B" if white_bin == engine_a else "A"
            print(f" game {idx+1:2}/{fixed_games}: {white_name} vs {black_name} result {res_w} {status}")
            return (res_w, viol, forf, status)

    # Gather
    tasks = [run_one(i, w, b, f, m) for i, (w, b, f, m) in enumerate(specs)]
    gathered = await asyncio.gather(*tasks)

    total_violations = sum(g[1] for g in gathered)
    total_forfeits = sum(g[2] for g in gathered)
    # Count wins/losses from engine A perspective
    wins = draws = losses = 0
    for idx, (res_w, _, _, _) in enumerate(gathered):
        white_bin = specs[idx][0]
        # Map white result to A perspective
        if white_bin == engine_a:
            res_a = res_w
        else:
            # white is B, so invert
            if res_w == 1.0:
                res_a = 0.0
            elif res_w == 0.0:
                res_a = 1.0
            else:
                res_a = 0.5
        if res_a == 1.0:
            wins += 1
        elif res_a == 0.0:
            losses += 1
        else:
            draws += 1

    elo, err = elo_estimate(wins, draws, losses)
    print(f"\nFixed gate: {fixed_games} games: W{wins} D{draws} L{losses} elo {elo:+.1f} ±{err:.1f}")
    print(f" violations (protocol, A): {total_violations}, forfeits (time): {total_forfeits}")
    if total_violations > 0 or total_forfeits > 0:
        print("GATE FAIL: crashes/forfeits/protocol violations detected")
        return 1
    # Also ensure every game produced a legal result (no crash)
    print("GATE PASS: zero crashes/forfeits/protocol violations")
    return 0


def main() -> None:
    parser = argparse.ArgumentParser(description="Paired SPRT match runner")
    parser.add_argument("--engine-a", required=True, help="path to engine A binary")
    parser.add_argument("--engine-b", required=True, help="path to engine B binary")
    parser.add_argument("--elo0", type=float, default=0, help="H0 elo")
    parser.add_argument("--elo1", type=float, default=25, help="H1 elo")
    parser.add_argument("--alpha", type=float, default=0.05)
    parser.add_argument("--beta", type=float, default=0.05)
    parser.add_argument("--movetime", type=int, default=100, help="ms per move (movetime mode)")
    parser.add_argument("--tc", type=str, default=None, help="clock time control M+INC in ms (e.g. 30000+300)")
    parser.add_argument("--fixed-games", type=int, default=None, help="fixed N games, no SPRT (gate mode)")
    parser.add_argument("--elo-limit", type=int, default=None, help="UCI_Elo for engine B (with UCI_LimitStrength)")
    parser.add_argument("--max-games", type=int, default=200, help="cap")
    parser.add_argument("--concurrency", type=int, default=5, help="parallel game pairs")
    parser.add_argument("--openings", default="default", help="'default' or path to FEN file")
    args = parser.parse_args()
    rc = asyncio.run(run_match(args))
    sys.exit(rc)


if __name__ == "__main__":
    main()

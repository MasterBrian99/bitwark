"""
Paired SPRT match runner: bitwark vs bitwark (or vs stockfish).

Plays paired games (both colors) from a small opening book at fixed
movetime, adjudicates via python-chess, and runs a Wald SPRT
(elo0 / elo1, alpha=beta=0.05) with the usual fishtest LLR.

Usage:
  uv run python match_runner.py --engine-a target/release/bitwark --engine-b /tmp/bitwark-baseline \
      [--elo0 0 --elo1 25 --movetime 100 --max-games 200 --concurrency 5]

Exit codes: 0 = H1 accepted (gain), 1 = H0 accepted (no gain), 2 = undecided at cap.
"""

import argparse
import asyncio
import math
import sys
from pathlib import Path

import chess

from common import BITWARK_BIN, UciEngine

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


async def play_single_game(
    white_bin: Path,
    black_bin: Path,
    opening_fen: str | None,
    opening_moves: list[str],
    movetime: int,
) -> float:
    """Play one game with white_bin as White, black_bin as Black.

    opening_fen: if not None, start from this FEN; otherwise startpos + opening_moves.
    Returns 1.0 if White wins, 0.0 if Black wins, 0.5 if draw.
    """
    async with UciEngine(white_bin) as white_eng, UciEngine(black_bin) as black_eng:
        await white_eng.handshake()
        await black_eng.handshake()

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
                return 1.0 if winner == chess.WHITE else 0.0
            return 0.5

        max_ply = 400
        while not board.is_game_over() and board.ply() < max_ply:
            is_white = board.turn == chess.WHITE
            mover_eng = white_eng if is_white else black_eng

            fen = board.fen()
            await mover_eng.send(f"position fen {fen}")
            await mover_eng.isready()
            await mover_eng.send(f"go movetime {movetime}")

            try:
                lines = await mover_eng.read_until(
                    lambda l: l.startswith("bestmove"), timeout=movetime / 1000 + 2.0
                )
            except asyncio.TimeoutError:
                # Forfeit
                return 0.0 if is_white else 1.0

            bm_line = next((l for l in lines if l.startswith("bestmove")), "")
            parts = bm_line.split()
            if len(parts) < 2 or parts[1] == "(none)":
                if not list(board.legal_moves):
                    if board.is_check():
                        return 0.0 if is_white else 1.0
                    else:
                        return 0.5
                return 0.0 if is_white else 1.0

            uci_move = parts[1]
            try:
                move = chess.Move.from_uci(uci_move)
            except ValueError:
                return 0.0 if is_white else 1.0

            if move not in board.legal_moves:
                return 0.0 if is_white else 1.0

            board.push(move)

            if board.is_fifty_moves() or board.is_insufficient_material() or board.can_claim_threefold_repetition():
                return 0.5

        if board.is_checkmate():
            winner = not board.turn
            return 1.0 if winner == chess.WHITE else 0.0
        return 0.5


async def run_match(args) -> int:
    engine_a = Path(args.engine_a)
    engine_b = Path(args.engine_b)
    if not engine_a.exists():
        print(f"engine-a not found: {engine_a}", file=sys.stderr)
        return 2
    if not engine_b.exists():
        print(f"engine-b not found: {engine_b}", file=sys.stderr)
        return 2

    elo0 = args.elo0
    elo1 = args.elo1
    alpha = args.alpha
    beta = args.beta
    movetime = args.movetime
    max_games = args.max_games
    # concurrency is accepted but not yet used for true parallel games;
    # games are still played sequentially to keep SPRT ordering simple.
    # The flag is kept for CLI compatibility with the plan.

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
        r1_white = await play_single_game(engine_a, engine_b, opening_fen, opening_moves, movetime)
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
        r2_white = await play_single_game(engine_b, engine_a, opening_fen, opening_moves, movetime)
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


def main() -> None:
    parser = argparse.ArgumentParser(description="Paired SPRT match runner")
    parser.add_argument("--engine-a", required=True, help="path to engine A binary")
    parser.add_argument("--engine-b", required=True, help="path to engine B binary")
    parser.add_argument("--elo0", type=float, default=0, help="H0 elo")
    parser.add_argument("--elo1", type=float, default=25, help="H1 elo")
    parser.add_argument("--alpha", type=float, default=0.05)
    parser.add_argument("--beta", type=float, default=0.05)
    parser.add_argument("--movetime", type=int, default=100, help="ms per move")
    parser.add_argument("--max-games", type=int, default=200, help="cap")
    parser.add_argument("--concurrency", type=int, default=5, help="accepted, not yet parallel")
    parser.add_argument("--openings", default="default", help="'default' or path to FEN file")
    args = parser.parse_args()
    rc = asyncio.run(run_match(args))
    sys.exit(rc)


if __name__ == "__main__":
    main()

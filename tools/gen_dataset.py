"""
Generate labeled quiet positions via stockfish self-play.

For each game, stockfish plays both sides at fixed depth from a random
opening. Every few plies the FEN is recorded, then labelled with
stockfish's static eval at a deeper fixed depth (cp). Output is
`fen|cp` per line.

Usage:
  uv run python gen_dataset.py --positions 20000 --depth 12 --concurrency 6 --out tools/data/quiet_positions.txt
"""

from __future__ import annotations

import argparse
import asyncio
import random
import sys
import time
from pathlib import Path

import chess

from common import STOCKFISH_BIN, UciEngine

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUT = REPO_ROOT / "tools" / "data" / "quiet_positions.txt"

# Use the same openings as match_runner for diversity
DEFAULT_OPENINGS = [
    [],
    ["e2e4", "e7e5"],
    ["e2e4", "c7c5"],
    ["d2d4", "d7d5"],
    ["d2d4", "g8f6"],
    ["g1f3", "d7d5"],
    ["c2c4", "e7e5"],
    ["g1f3", "g8f6"],
    ["e2e4", "e7e6"],
    ["d2d4", "c7c5"],
]

async def stockfish_eval(fen: str, depth: int, sem: asyncio.Semaphore) -> int | None:
    """Return cp from White perspective for fen at fixed depth, or None on error."""
    async with sem:
        async with UciEngine(STOCKFISH_BIN) as eng:
            await eng.handshake()
            await eng.send(f"position fen {fen}")
            await eng.isready()
            await eng.send(f"go depth {depth}")
            lines = await eng.read_until(lambda l: l.startswith("bestmove"), timeout=10)
            # Find last info with score cp
            cp = None
            for line in lines:
                if "score cp" in line:
                    parts = line.split()
                    try:
                        idx = parts.index("cp")
                        cp = int(parts[idx + 1])
                    except Exception:
                        pass
                elif "score mate" in line:
                    # Mate scores — skip this position (unstable)
                    cp = None
                    break
            if cp is None:
                return None
            # Convert to White perspective (stockfish reports from side to move)
            board = chess.Board(fen)
            if board.turn == chess.BLACK:
                cp = -cp
            return cp


async def play_game(opening: list[str], play_depth: int, sem: asyncio.Semaphore) -> list[str]:
    """Play one self-play game at play_depth, return list of FENs (one per collected ply)."""
    async with sem:
        async with UciEngine(STOCKFISH_BIN) as eng:
            await eng.handshake()
            board = chess.Board()
            # 8 random plies to break determinism (fixed openings at fixed depth are deterministic)
            for _ in range(8):
                if board.is_game_over():
                    break
                moves = list(board.legal_moves)
                random.shuffle(moves)
                chosen = None
                for mv in moves:
                    board.push(mv)
                    if not board.is_check():
                        chosen = mv
                        break
                    board.pop()
                if chosen is None:
                    # all moves give check, just push a random one
                    board.push(random.choice(list(board.legal_moves)))
            for mv in opening:
                try:
                    board.push_uci(mv)
                except Exception:
                    break
            fens: list[str] = []
            max_ply = 120
            ply = board.ply()
            while ply < max_ply and not board.is_game_over():
                # Record every 4 plies after move 8, skip checks
                if ply >= 8 and ply % 4 == 0 and not board.is_check():
                    # Only quiet positions: skip if side to move is in check (already) or has many captures?
                    # For now, record all non-check positions.
                    fens.append(board.fen())
                # Ask stockfish for bestmove at play_depth
                await eng.send(f"position fen {board.fen()}")
                await eng.isready()
                await eng.send(f"go depth {play_depth}")
                try:
                    lines = await eng.read_until(lambda l: l.startswith("bestmove"), timeout=5)
                except asyncio.TimeoutError:
                    break
                bm = next((l.split()[1] for l in lines if l.startswith("bestmove") and len(l.split()) >= 2), None)
                if not bm or bm == "(none)":
                    break
                try:
                    board.push_uci(bm)
                except Exception:
                    break
                ply += 1
                # Early stop if game already decided
                if board.is_checkmate() or board.is_stalemate() or board.is_insufficient_material():
                    break
            return fens


async def main() -> None:
    parser = argparse.ArgumentParser(description="Generate SF-labeled quiet positions")
    parser.add_argument("--positions", type=int, default=5000, help="target number of labeled positions")
    parser.add_argument("--play-depth", type=int, default=8, help="depth for self-play moves")
    parser.add_argument("--eval-depth", type=int, default=12, help="depth for labeling eval")
    parser.add_argument("--concurrency", type=int, default=6, help="parallel games / eval workers")
    parser.add_argument("--out", type=str, default=str(DEFAULT_OUT), help="output file")
    args = parser.parse_args()

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    sem_games = asyncio.Semaphore(args.concurrency)
    sem_eval = asyncio.Semaphore(args.concurrency * 2)

    collected: list[str] = []
    game_idx = 0
    # Pass 1: play games until we have enough raw FENs
    raw_fens: list[str] = []
    print(f"Pass 1: playing games at depth {args.play_depth} to collect {args.positions} positions")
    t0 = time.time()
    while len(raw_fens) < args.positions * 2:  # oversample, many will be filtered during labeling
        batch = []
        for _ in range(args.concurrency * 2):
            opening = random.choice(DEFAULT_OPENINGS)
            batch.append(play_game(opening, args.play_depth, sem_games))
        results = await asyncio.gather(*batch)
        for fens in results:
            raw_fens.extend(fens)
        print(f"  games batch {game_idx}: total raw {len(raw_fens)}/{args.positions}")
        game_idx += 1
        if len(raw_fens) >= args.positions * 3:
            break
        if time.time() - t0 > 600:
            print("  timeout in pass 1")
            break
        # Shuffle to avoid bias
        random.shuffle(raw_fens)
        raw_fens = raw_fens[: args.positions * 3]

    # Deduplicate and shuffle
    raw_fens = list(dict.fromkeys(raw_fens))
    random.shuffle(raw_fens)
    raw_fens = raw_fens[: args.positions * 2]
    print(f"Pass 1 done: {len(raw_fens)} raw FENs")

    # Pass 2: label with SF eval
    print(f"Pass 2: labeling {args.positions} positions at depth {args.eval_depth}")
    labeled: list[str] = []
    # Process in batches
    idx = 0
    while len(labeled) < args.positions and idx < len(raw_fens):
        batch_fens = raw_fens[idx : idx + args.concurrency * 4]
        idx += len(batch_fens)
        tasks = [stockfish_eval(fen, args.eval_depth, sem_eval) for fen in batch_fens]
        cps = await asyncio.gather(*tasks)
        for fen, cp in zip(batch_fens, cps):
            if cp is None:
                continue
            if abs(cp) > 1500:
                continue
            labeled.append(f"{fen}|{cp}")
            if len(labeled) >= args.positions:
                break
        print(f"  labeled {len(labeled)}/{args.positions} (processed {idx}/{len(raw_fens)})")

    # Write
    with open(out_path, "w") as f:
        for line in labeled[: args.positions]:
            f.write(line + "\n")
    print(f"Wrote {min(len(labeled), args.positions)} positions to {out_path}")


if __name__ == "__main__":
    asyncio.run(main())

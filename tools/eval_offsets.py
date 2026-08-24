"""
Compute per-position eval offsets (other-terms interpolated value) for residual Texel tuning.

For each fen|cp line, query bitwark eval breakdown and compute:
  offset = interpolated(PawnStruct+Mobility+RookFiles+BishopPair+KingSafety)
which is constant w.r.t. PSQT+material tuning (tables.rs).

Writes fen|cp|offset per line.

Usage:
  uv run python eval_offsets.py --data /tmp/quiet_100k.txt --out /tmp/quiet_100k.offsets --engine ../target/release/bitwark --concurrency 12
"""
from __future__ import annotations
import argparse
import asyncio
import re
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).parent))
from common import UciEngine, BITWARK_BIN

# Terms that are PSQT+material (tuned). All others are offset.
TUNED_TERMS = {"Material", "PieceSq"}

async def offset_for_fen(fen: str, engine_path: Path, sem: asyncio.Semaphore) -> int | None:
    async with sem:
        async with UciEngine(engine_path) as eng:
            await eng.handshake()
            await eng.send(f"position fen {fen}")
            await eng.isready()
            await eng.send("eval")
            # Read until Total evaluation line (white side) - the breakdown lines precede it
            lines = await eng.read_until(lambda l: "Total evaluation:" in l and "white side" in l, timeout=5)
            # Parse the game phase and per-term rows
            phase = None
            per_term = {}
            total_mg = total_eg = None
            for line in lines:
                if line.strip().startswith("Phase:"):
                    m = re.search(r"Phase:\s*(\d+)/24", line)
                    if m:
                        phase = int(m.group(1))
                # Rows like " Material |    0    0 |     0"  - parse name and mg eg
                # Format from session.rs: f" {name:9} | {mg:5} {eg:5} | {total:5}"
                # So split by | and spaces
                if "|" in line and any(name in line for name in ["Material","PieceSq","PawnStruct","Mobility","RookFiles","BishopPair","KingSafety","KingShield","KingAttack","Passers","KingAct","Outposts","BadBishop","Trapped","Rook7th"]):
                    parts = line.split("|")
                    if len(parts) >= 2:
                        name = parts[0].strip()
                        # second part: mg eg
                        mg_eg = parts[1].strip().split()
                        if len(mg_eg) >= 2:
                            try:
                                mg = int(mg_eg[0]); eg = int(mg_eg[1])
                            except:
                                continue
                            per_term[name] = (mg, eg)
                if line.strip().startswith("Total") and "|" in line and "Material" not in line:
                    # Total row
                    parts = line.split("|")
                    if len(parts) >= 2:
                        mg_eg = parts[1].strip().split()
                        if len(mg_eg) >= 2:
                            try:
                                total_mg = int(mg_eg[0]); total_eg = int(mg_eg[1])
                            except:
                                pass
            if phase is None:
                return None
            # Compute offset interpolated
            off_mg = 0
            off_eg = 0
            for name, (mg, eg) in per_term.items():
                if name in TUNED_TERMS:
                    continue
                # also skip Total row if captured
                if name == "Total":
                    continue
                off_mg += mg
                off_eg += eg
            # Also handle case where per_term parsing missed some (fallback: total - tuned)
            # If we have total_mg/eg and tuned sums, verify
            # But off as summed above is fine
            offset = (off_mg * phase + off_eg * (24 - phase)) // 24
            return offset

async def main():
    ap = argparse.ArgumentParser(description="Compute bitwark eval offsets for residual tuning")
    ap.add_argument("--data", type=str, required=True, help="input fen|cp file")
    ap.add_argument("--out", type=str, required=True, help="output fen|cp|offset file")
    ap.add_argument("--engine", type=str, default=str(BITWARK_BIN), help="bitwark binary")
    ap.add_argument("--concurrency", type=int, default=12, help="parallel engines")
    ap.add_argument("--limit", type=int, default=None, help="limit positions")
    args = ap.parse_args()

    in_path = Path(args.data)
    out_path = Path(args.out)
    engine_path = Path(args.engine)

    lines = []
    with open(in_path) as f:
        for line in f:
            line=line.strip()
            if not line or line.startswith("#") or "|" not in line:
                continue
            fen = line.rsplit("|",1)[0].strip()
            cp = line.rsplit("|",1)[1].strip()
            lines.append((fen, cp))
            if args.limit and len(lines) >= args.limit:
                break
    print(f"Loaded {len(lines)} positions from {in_path}")

    sem = asyncio.Semaphore(args.concurrency)
    # Process in batches to avoid too many concurrent engines? Use gather with sem
    offsets = []
    # Use chunks
    batch_size = args.concurrency * 4
    for i in range(0, len(lines), batch_size):
        batch = lines[i:i+batch_size]
        tasks = [offset_for_fen(fen, engine_path, sem) for fen,_ in batch]
        results = await asyncio.gather(*tasks)
        for (fen, cp), off in zip(batch, results):
            if off is None:
                off = 0
            offsets.append(f"{fen}|{cp}|{off}")
        print(f"  {min(i+batch_size, len(lines))}/{len(lines)} offsets computed")
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with open(out_path, "w") as out:
        for line in offsets:
            out.write(line + "\n")
    print(f"Wrote {len(offsets)} lines to {out_path}")

if __name__ == "__main__":
    asyncio.run(main())

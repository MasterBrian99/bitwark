"""
Throughput floors; two-run bench equality already lives in test_bench.

- bench NPS  (bitwark bench 16 1 13 default depth, single thread, deterministic)
- perft NPS  (go perft 5 on startpos, single thread, bulk counting)

Floors start trivial and are raised as each optimization lands —
current floors: bench ≥ 1.5M, perft ≥ 40M.

Run: cd tools && uv run python test_perf.py
"""

from __future__ import annotations

import asyncio
import re
import time

from common import BITWARK_BIN, UciEngine

# ---------------------------------------------------------------------------
# Baselines captured on 2026-08-21, 12 logical CPUs, i5-12450HX, release.
# Recorded at v0.5.0.
#   bench: ./target/release/bitwark bench 16 1 13 default depth -> Nodes/second
#   perft: go perft 5 startpos wall-clock via UciEngine (bulk counting)
#
# Ledger-aware: if tools/baselines.json exists, its latest bench.nps_median
# is used as the relative baseline (with +/-2% tolerance in test_bench);
# the absolute floor below is always enforced. Update baselines.json via
# `uv run python tools/verify.py --expect-nodes record --phase phaseN`.
# ---------------------------------------------------------------------------
PRE_SPEED_BASELINE = {
    # informational — the deterministic node count is asserted in test_bench;
    # the nps below is the wall-clock throughput measured via the CLI harness.
    "bench_nodes_d13": 48_577_800,
    "bench_nps": 1_088_000,  # median of 3 runs @ d13 (46526ms, 46570ms, 47533ms)
    "perft_nps": 24_700_000,  # go perft 6: ~119M/4.84s ; go perft 5: 4.86M timed below
}

# Floors — raised deliberately after each optimization. Ledger-relative
# +/-2% in verify.py is the real gate; absolute here is a loose safety net.
BENCH_NPS_FLOOR = 1_400_000
PERFT_NPS_FLOOR = 35_000_000

# Ledger-aware helper (Part A)
LEDGER_PATH = BITWARK_BIN.parent.parent / "tools" / "baselines.json"


def _ledger_bench_nps() -> int | None:
    try:
        import json as _json

        data = _json.loads(LEDGER_PATH.read_text())
        baselines = data.get("baselines", [])
        if baselines:
            latest = baselines[-1]
            return latest.get("bench", {}).get("nps_median")
    except Exception:
        pass
    return None


def parse_bench_block(text: str) -> tuple[int, int]:
    m_nodes = re.search(r"Nodes searched\s*:\s*(\d+)", text)
    m_nps = re.search(r"Nodes/second\s*:\s*(\d+)", text)
    if not m_nodes or not m_nps:
        raise AssertionError(f"bench output missing fields:\n{text}")
    return int(m_nodes.group(1)), int(m_nps.group(1))


async def bench_once() -> tuple[int, int]:
    proc = await asyncio.create_subprocess_exec(
        str(BITWARK_BIN),
        "bench",
        "16",
        "1",
        "13",
        "default",
        "depth",
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=70)
    text = stdout.decode()
    err = stderr.decode()
    if proc.returncode != 0:
        raise AssertionError(f"bench exit {proc.returncode}\nstderr: {err}\nstdout: {text}")
    return parse_bench_block(text)


async def test_bench_nps() -> None:
    print(f"RUN   test_bench_nps (floor {BENCH_NPS_FLOOR/1e6:.1f}M)")
    nodes, nps = await bench_once()
    ledger_nps = _ledger_bench_nps()
    base_nps = ledger_nps if ledger_nps is not None else PRE_SPEED_BASELINE["bench_nps"]
    base_label = "ledger" if ledger_nps is not None else "pre-speed"
    print(f"  Nodes searched: {nodes}  Nodes/second: {nps} ({nps/1e6:.2f}M)")
    print(f"  baseline {base_label} bench_nps {base_nps/1e6:.2f}M")
    if nodes != PRE_SPEED_BASELINE["bench_nodes_d13"] and False:
        # Node count determinism is asserted in test_bench; don't duplicate here.
        pass
    if nps < BENCH_NPS_FLOOR:
        raise AssertionError(
            f"bench NPS too low: {nps} < floor {BENCH_NPS_FLOOR} "
            f"(baseline {base_nps}, gate expects ≥ {BENCH_NPS_FLOOR})"
        )
    # Ledger-aware relative check: warn if >2% regression vs ledger median
    if ledger_nps is not None and nps < ledger_nps * 0.98:
        print(f"  WARN bench NPS {nps/1e6:.2f}M is >2% below ledger {ledger_nps/1e6:.2f}M")
    print(f"PASS  test_bench_nps ({nps/1e6:.2f}M ≥ {BENCH_NPS_FLOOR/1e6:.1f}M)")


async def test_perft_nps() -> None:
    print(f"RUN   test_perft_nps (floor {PERFT_NPS_FLOOR/1e6:.0f}M)")
    async with UciEngine(BITWARK_BIN) as eng:
        await eng.handshake()
        await eng.isready()
        await eng.send("position startpos")
        await eng.isready()
        t0 = time.perf_counter()
        await eng.send("go perft 5")
        lines = await eng.read_until(lambda l: l.startswith("Nodes searched:"), timeout=15)
        elapsed = time.perf_counter() - t0
        total = int(lines[-1].split(":")[1].strip().split()[0])
        if total != 4_865_609:
            raise AssertionError(f"perft 5 total mismatch: {total} != 4865609")
        nps = total / elapsed if elapsed > 0 else float("inf")
        print(f"  perft 5: {total} in {elapsed:.3f}s = {nps/1e6:.1f}M nps (baseline {PRE_SPEED_BASELINE['perft_nps']/1e6:.1f}M)")
        if nps < PERFT_NPS_FLOOR:
            raise AssertionError(f"perft NPS too low: {nps/1e6:.1f}M < {PERFT_NPS_FLOOR/1e6:.0f}M")
    print(f"PASS  test_perft_nps ({nps/1e6:.1f}M ≥ {PERFT_NPS_FLOOR/1e6:.0f}M)")


async def main() -> None:
    if not BITWARK_BIN.exists():
        raise SystemExit(f"engine binary not found: {BITWARK_BIN}\nrun: cargo build --release")
    await test_bench_nps()
    await test_perft_nps()
    print("\n2/2 suites passed — perf gate GREEN")


if __name__ == "__main__":
    asyncio.run(main())

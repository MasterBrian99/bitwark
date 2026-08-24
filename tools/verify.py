#!/usr/bin/env python3
"""
Unified verification harness for Bitwark (Part A, Phase-0).

Runs the full A.4 verification protocol in one command:

  cargo test --release  (0 failures)
  perft gates           (startpos d5 = 4865609, Kiwipete d4 = 4085603)
  bench determinism     (3 runs @ 16/1/13, nodes identical)
  bench NPS floor       (vs baselines.json, +/-2% + absolute floor)
  python tools suite    (each test_*.py via uv)

Usage:
  uv run python tools/verify.py                 # full suite, 3 bench runs, compare vs ledger
  uv run python tools/verify.py --quick         # skip heavy python tests (search/conformance long)
  uv run python tools/verify.py --bench-runs 1  # single bench run (fast)
  uv run python tools/verify.py --expect-nodes exact   # require nodes == ledger (Phase 1 guardrail)
  uv run python tools/verify.py --expect-nodes record --phase phase0  # record new baseline

Exit 0 = all checks pass, 1 = any failure.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
BITWARK_BIN = REPO_ROOT / "target" / "release" / "bitwark"
BASELINES_JSON = REPO_ROOT / "tools" / "baselines.json"

# Absolute floor (conservative; ledger-relative +/-2% is the real gate)
# Bench fluctuates 1.45-1.55M on i5-12450HX; 1.4M leaves headroom for noise.
ABSOLUTE_BENCH_FLOOR = 1_400_000
ABSOLUTE_PERFT_FLOOR = 35_000_000

PERFT_EXPECT = {
    "startpos d5": 4_865_609,
    "kiwipete d4": 4_085_603,
}
KIWIPETE_FEN = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1"

# Bench harness defaults (must match src/bench.rs DEFAULT_FENS suite)
BENCH_TT = 16
BENCH_THREADS = 1
BENCH_DEPTH = 13
BENCH_TOLERANCE = 0.02  # +/-2% NPS


def eprint(*a, **kw):
    print(*a, file=sys.stderr, **kw)


def run_cmd(cmd: list[str], timeout: float = 120, cwd: Path | None = None) -> tuple[int, str, str]:
    """Run cmd, return (returncode, stdout, stderr)."""
    try:
        proc = subprocess.run(
            cmd,
            cwd=str(cwd or REPO_ROOT),
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        return proc.returncode, proc.stdout, proc.stderr
    except subprocess.TimeoutExpired:
        return 124, "", f"timeout after {timeout}s: {' '.join(cmd)}"


def check_bin_exists() -> bool:
    if not BITWARK_BIN.exists():
        eprint(f"ERROR: engine binary not found: {BITWARK_BIN}")
        eprint("  run: cargo build --release")
        return False
    return True


# ---------------------------------------------------------------------------
# Baselines ledger helpers
# ---------------------------------------------------------------------------

def load_baselines() -> dict:
    if not BASELINES_JSON.exists():
        return {"version": 1, "baselines": []}
    try:
        return json.loads(BASELINES_JSON.read_text())
    except Exception as ex:
        eprint(f"WARNING: failed to read {BASELINES_JSON}: {ex}, using empty")
        return {"version": 1, "baselines": []}


def get_latest_baseline(data: dict) -> dict | None:
    baselines = data.get("baselines", [])
    if not baselines:
        return None
    return baselines[-1]


def get_cpu_info() -> str:
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if "model name" in line:
                    return line.split(":", 1)[1].strip()
    except Exception:
        pass
    return "unknown"


def get_rustc_version() -> str:
    rc, out, _ = run_cmd(["rustc", "--version"], timeout=5)
    return out.strip() if rc == 0 else "unknown"


# ---------------------------------------------------------------------------
# Steps
# ---------------------------------------------------------------------------

def step_cargo_test() -> bool:
    print("=" * 60)
    print("STEP cargo test --release")
    print("=" * 60)
    rc, out, err = run_cmd(["cargo", "test", "--release"], timeout=300)
    # cargo test prints to stderr via test harness; combine
    combined = out + "\n" + err
    # Look for failures
    if rc != 0:
        print(combined[-4000:])
        eprint("FAIL cargo test --release (exit {})".format(rc))
        return False
    # Also check for "FAILED" in output (paranoid)
    if "FAILED" in combined or "failed" in combined.lower() and "0 failed" not in combined:
        # Use regex for "X passed; Y failed"
        if re.search(r"\b[1-9]\d* failed", combined):
            print(combined[-4000:])
            eprint("FAIL cargo test --release (failures in output)")
            return False
    print("PASS cargo test --release")
    return True


def parse_perft_output(text: str) -> int | None:
    m = re.search(r"Nodes searched:\s*(\d+)", text)
    if m:
        return int(m.group(1))
    # fallback: last number after colon
    for line in reversed(text.splitlines()):
        if "Nodes searched" in line:
            parts = line.split(":")
            if len(parts) >= 2:
                try:
                    return int(parts[1].strip().split()[0])
                except Exception:
                    pass
    return None


def step_perft() -> bool:
    print("=" * 60)
    print("STEP perft gates")
    print("=" * 60)
    if not check_bin_exists():
        return False

    # Gate 1: startpos d5 via CLI perft
    cases = [
        (["perft", "5", "--fen", "startpos"], PERFT_EXPECT["startpos d5"], "startpos d5"),
        (["perft", "4", "--fen", KIWIPETE_FEN], PERFT_EXPECT["kiwipete d4"], "kiwipete d4"),
    ]
    ok = True
    for args, expected, label in cases:
        cmd = [str(BITWARK_BIN)] + args
        print(f"  perft {label}: {' '.join(cmd)}")
        rc, out, err = run_cmd(cmd, timeout=30)
        text = out + "\n" + err
        if rc != 0:
            eprint(f"  FAIL perft {label}: exit {rc}\n{text[-2000:]}")
            ok = False
            continue
        nodes = parse_perft_output(text)
        if nodes is None:
            eprint(f"  FAIL perft {label}: could not parse nodes from:\n{text[-1000:]}")
            ok = False
            continue
        if nodes != expected:
            eprint(f"  FAIL perft {label}: got {nodes} expected {expected}")
            ok = False
            continue
        print(f"  PASS perft {label}: {nodes} == {expected}")

    # Also check bulk-count perft via UciEngine go perft (optional, not failing if CLI passed)
    if ok:
        print("PASS perft gates")
    else:
        eprint("FAIL perft gates")
    return ok


def parse_bench_output(text: str) -> tuple[int, int] | None:
    m_nodes = re.search(r"Nodes searched\s*:\s*(\d+)", text)
    m_nps = re.search(r"Nodes/second\s*:\s*(\d+)", text)
    if not m_nodes or not m_nps:
        return None
    return int(m_nodes.group(1)), int(m_nps.group(1))


def step_bench(bench_runs: int, expect_nodes: str) -> tuple[bool, list[dict]]:
    print("=" * 60)
    print(f"STEP bench ({bench_runs} runs @ {BENCH_TT}/{BENCH_THREADS}/{BENCH_DEPTH})")
    print("=" * 60)
    if not check_bin_exists():
        return False, []

    if shutil.which("cargo") is None:
        eprint("WARNING: cargo not found, skipping bench")
        return False, []

    ledger = load_baselines()
    latest = get_latest_baseline(ledger)
    expected_nodes = latest["bench"]["nodes"] if latest and "bench" in latest else None

    results: list[dict] = []
    nodes_list: list[int] = []
    nps_list: list[int] = []
    ok = True

    for i in range(bench_runs):
        print(f"  bench run {i+1}/{bench_runs} ...", flush=True)
        cmd = [str(BITWARK_BIN), "bench", str(BENCH_TT), str(BENCH_THREADS), str(BENCH_DEPTH), "default", "depth"]
        rc, out, err = run_cmd(cmd, timeout=90)
        text = out + "\n" + err
        if rc != 0:
            eprint(f"  FAIL bench run {i+1}: exit {rc}\n{text[-2000:]}")
            ok = False
            break
        parsed = parse_bench_output(text)
        if not parsed:
            eprint(f"  FAIL bench run {i+1}: could not parse output\n{text[-1000:]}")
            ok = False
            break
        nodes, nps = parsed
        # also parse time if available
        m_time = re.search(r"Total time \(ms\)\s*:\s*(\d+)", text)
        tms = int(m_time.group(1)) if m_time else 0
        print(f"    run {i+1}: nodes={nodes} nps={nps} time={tms}ms")
        nodes_list.append(nodes)
        nps_list.append(nps)
        results.append({"nodes": nodes, "nps": nps, "time_ms": tms})

    if not ok:
        return False, results

    # Determinism check (single-thread bench must be identical)
    if len(set(nodes_list)) != 1:
        eprint(f"  FAIL bench determinism: nodes differed across runs: {nodes_list}")
        ok = False
    else:
        print(f"  PASS bench determinism: {nodes_list[0]} identical across {bench_runs} runs")

    # Expect-nodes mode
    if expect_nodes == "exact":
        if expected_nodes is None:
            eprint("  FAIL --expect-nodes exact but no ledger baseline to compare (run with --expect-nodes record first)")
            ok = False
        elif nodes_list[0] != expected_nodes:
            eprint(f"  FAIL nodes exact: got {nodes_list[0]} expected {expected_nodes} (ledger)")
            eprint("  This phase requires bit-identical search decisions; investigate before landing.")
            ok = False
        else:
            print(f"  PASS nodes exact: {nodes_list[0]} == ledger {expected_nodes}")

    # NPS floor check (median vs ledger +/-2% and absolute floor)
    if nps_list:
        nps_sorted = sorted(nps_list)
        nps_median = nps_sorted[len(nps_sorted) // 2]
        print(f"  bench median NPS: {nps_median} ({nps_median/1e6:.2f}M)")

        # Absolute floor
        if nps_median < ABSOLUTE_BENCH_FLOOR:
            eprint(f"  FAIL bench NPS floor: {nps_median} < absolute {ABSOLUTE_BENCH_FLOOR}")
            ok = False
        else:
            print(f"  PASS bench absolute floor: {nps_median} >= {ABSOLUTE_BENCH_FLOOR}")

        # Relative vs ledger
        if latest and "bench" in latest and "nps_median" in latest["bench"]:
            ledger_nps = latest["bench"]["nps_median"]
            if ledger_nps:
                delta = (nps_median - ledger_nps) / ledger_nps
                print(f"  vs ledger {ledger_nps} ({ledger_nps/1e6:.2f}M): delta {delta*100:+.1f}% (tolerance +/-{BENCH_TOLERANCE*100:.0f}%)")
                if delta < -BENCH_TOLERANCE - 1e-9:
                    eprint(f"  FAIL bench NPS regression > tolerance: {delta*100:.1f}% (>{BENCH_TOLERANCE*100:.0f}%)")
                    ok = False
                elif delta < 0:
                    print(f"  WARN bench NPS slightly down {delta*100:.1f}% but within tolerance")
                else:
                    print(f"  PASS bench NPS vs ledger (within tolerance)")

        # Also report if ledger missing
        if latest is None:
            print("  (no ledger baseline; skipping relative NPS check)")

    if ok:
        print("PASS bench suite")
    else:
        eprint("FAIL bench suite")
    return ok, results


def step_tools_suite(quick: bool) -> bool:
    print("=" * 60)
    print(f"STEP python tools suite ({'quick' if quick else 'full'})")
    print("=" * 60)

    # Discover test_*.py that are real test suites (have a main() / if __name__)
    tools_dir = REPO_ROOT / "tools"
    # Explicit list in preferred order (heavy last)
    suites_full = [
        "test_perft.py",
        "test_fen.py",
        "test_eval.py",
        "test_bench.py",
        "test_perf.py",
        "test_uci.py",
        "test_conformance.py",
        "test_search.py",
    ]
    suites_quick = [
        "test_perft.py",
        "test_fen.py",
        "test_eval.py",
        "test_bench.py",
        "test_perf.py",
        "test_uci.py",
    ]
    suites = suites_quick if quick else suites_full

    ok = True
    for name in suites:
        path = tools_dir / name
        if not path.exists():
            print(f"  SKIP {name} (not found)")
            continue
        print(f"  run {name} ...", flush=True)
        # Prefer `uv run` if available
        if shutil.which("uv"):
            cmd = ["uv", "run", "python", name]
            cwd = tools_dir
        else:
            cmd = [sys.executable, name]
            cwd = tools_dir
        rc, out, err = run_cmd(cmd, timeout=180, cwd=cwd)
        text = out + "\n" + err
        # Heuristic pass: exit 0 and contains PASS or no FAIL
        if rc != 0:
            eprint(f"  FAIL {name}: exit {rc}")
            print(text[-3000:])
            ok = False
            continue
        if "FAIL" in text and "0 failed" not in text.lower() and "FAIL" in text.split("PASS")[0] if "PASS" in text else True:
            # More robust: if text contains "FAIL" and not "0 failed", treat as fail
            # But some suites print "FAIL" in comments? Check.
            pass
        # Check for explicit FAIL marker from our suites: "FAIL  test_"
        if "FAIL  " in text:
            eprint(f"  FAIL {name}: suite reported FAIL")
            print(text[-3000:])
            ok = False
            continue
        print(f"  PASS {name}")
        # Optionally print tail PASS line
        for line in text.splitlines()[-4:]:
            if line.strip():
                print(f"    {line}")

    if ok:
        print("PASS python tools suite")
    else:
        eprint("FAIL python tools suite")
    return ok


# ---------------------------------------------------------------------------
# Baseline recording
# ---------------------------------------------------------------------------

def record_baseline(phase: str, bench_results: list[dict], perft_ok: bool) -> bool:
    ledger = load_baselines()
    # Compute median NPS
    if not bench_results:
        eprint("ERROR: no bench results to record")
        return False
    nodes = bench_results[0]["nodes"]  # all identical (determinism checked)
    nps_list = sorted(r["nps"] for r in bench_results)
    nps_median = nps_list[len(nps_list) // 2]
    # Also try perft NPS if available? Use bench time for now.

    entry = {
        "phase": phase,
        "date": datetime.now(timezone.utc).isoformat(),
        "cpu": get_cpu_info(),
        "rustc": get_rustc_version(),
        "git_tag": f"{phase}-baseline" if not phase.startswith("phase") else f"{phase}-baseline",
        "bench": {
            "tt": BENCH_TT,
            "threads": BENCH_THREADS,
            "depth": BENCH_DEPTH,
            "suite": "default",
            "nodes": nodes,
            "nps_median": nps_median,
            "runs": bench_results,
        },
        "perft": {
            "startpos_d5": PERFT_EXPECT["startpos d5"],
            "kiwipete_d4": PERFT_EXPECT["kiwipete d4"],
            "ok": perft_ok,
        },
    }
    # Upsert: replace if same phase exists, else append
    baselines = ledger.get("baselines", [])
    replaced = False
    for i, b in enumerate(baselines):
        if b.get("phase") == phase:
            baselines[i] = entry
            replaced = True
            break
    if not replaced:
        baselines.append(entry)
    ledger["baselines"] = baselines
    # Write atomically
    tmp = BASELINES_JSON.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(ledger, indent=2) + "\n")
    tmp.replace(BASELINES_JSON)
    print(f"{'Updated' if replaced else 'Recorded'} baseline {phase}: nodes={nodes} nps_median={nps_median} -> {BASELINES_JSON}")
    return True


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> int:
    p = argparse.ArgumentParser(description="Unified verification harness (Part A)")
    p.add_argument("--quick", action="store_true", help="skip heavy python suites (search/conformance)")
    p.add_argument("--bench-runs", type=int, default=3, help="bench repetitions for median (default 3)")
    p.add_argument("--expect-nodes", choices=["ignore", "exact", "record"], default="ignore",
                   help="how to handle bench node signature: ignore (default), exact (require == ledger), record (write ledger)")
    p.add_argument("--phase", type=str, default=None, help="phase name for --expect-nodes record (e.g. phase0)")
    p.add_argument("--skip-cargo-test", action="store_true", help="skip cargo test step")
    p.add_argument("--skip-perft", action="store_true", help="skip perft gates")
    p.add_argument("--skip-bench", action="store_true", help="skip bench suite")
    p.add_argument("--skip-tools", action="store_true", help="skip python tools suite")
    args = p.parse_args()

    if args.expect_nodes == "record" and not args.phase:
        eprint("ERROR: --expect-nodes record requires --phase NAME")
        return 2
    if args.bench_runs < 1:
        eprint("ERROR: --bench-runs must be >=1")
        return 2

    print("Bitwark verify — Part A harness")
    print(f"  bin: {BITWARK_BIN} ({'found' if BITWARK_BIN.exists() else 'MISSING'})")
    print(f"  ledger: {BASELINES_JSON} ({'found' if BASELINES_JSON.exists() else 'missing (will be created on record)'})")
    ldr = load_baselines()
    latest = get_latest_baseline(ldr)
    if latest:
        print(f"  latest ledger: {latest.get('phase')} nodes={latest.get('bench', {}).get('nodes')} nps={latest.get('bench', {}).get('nps_median')}")
    print(f"  quick={args.quick} bench_runs={args.bench_runs} expect_nodes={args.expect_nodes} phase={args.phase}")
    print()

    overall_ok = True
    bench_results: list[dict] = []
    perft_ok = True

    if not args.skip_cargo_test:
        if not step_cargo_test():
            overall_ok = False
        print()
    else:
        print("SKIP cargo test (--skip-cargo-test)\n")

    if not args.skip_perft:
        perft_ok = step_perft()
        if not perft_ok:
            overall_ok = False
        print()
    else:
        print("SKIP perft (--skip-perft)\n")

    if not args.skip_bench:
        bench_ok, bench_results = step_bench(args.bench_runs, args.expect_nodes)
        if not bench_ok:
            overall_ok = False
        print()
    else:
        print("SKIP bench (--skip-bench)\n")

    if not args.skip_tools:
        if not step_tools_suite(quick=args.quick):
            overall_ok = False
        print()
    else:
        print("SKIP python tools (--skip-tools)\n")

    # Record baseline if requested and overall passed (or bench at least passed)
    if args.expect_nodes == "record":
        if not bench_results:
            eprint("ERROR: --expect-nodes record but no bench results (bench was skipped or failed)")
            overall_ok = False
        else:
            # Record even if other steps had warnings, but bench must have succeeded
            bench_ok = len(bench_results) == args.bench_runs and len(set(r["nodes"] for r in bench_results)) == 1
            if bench_ok:
                if not record_baseline(args.phase, bench_results, perft_ok):
                    overall_ok = False
            else:
                eprint("ERROR: bench results not suitable for recording (non-deterministic or incomplete)")
                overall_ok = False

    print("=" * 60)
    if overall_ok:
        print("VERIFY PASS — all gates green")
        return 0
    else:
        eprint("VERIFY FAIL — one or more gates failed")
        return 1


if __name__ == "__main__":
    sys.exit(main())

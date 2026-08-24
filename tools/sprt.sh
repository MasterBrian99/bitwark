#!/usr/bin/env bash
# sprt.sh — canonical SPRT wrapper for Part A (gain-gate 0/+5, movetime 100ms)
#
# Usage:
#   ./tools/sprt.sh <baseline-tag-or-binary> [max-games] [openings]
#
# Examples:
#   ./tools/sprt.sh phase0              # compare HEAD vs tools/baselines/phase0
#   ./tools/sprt.sh /tmp/old-bitwark 200
#   ./tools/sprt.sh phase0 200 tools/data/openings.txt --sequential
#
# Presets (locked per Part A decisions):
#   elo0=0 elo1=5 movetime=100 concurrency=6 openings=tools/data/openings.txt
# Override via env: ELO0, ELO1, MOVETIME, CONCURRENCY, OPENINGS, TC
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BASELINE_ARG="${1:-}"
MAX_GAMES="${2:-200}"
OPENINGS_ARG="${3:-tools/data/openings.txt}"

if [[ -z "$BASELINE_ARG" ]]; then
  echo "Usage: $0 <baseline-tag-or-binary> [max-games] [openings]" >&2
  echo "  baseline-tag: phase0, phase1, ... (looks in tools/baselines/)" >&2
  echo "  or path to binary: /tmp/bitwark-old" >&2
  exit 2
fi

# Resolve baseline binary
if [[ -f "$BASELINE_ARG" && -x "$BASELINE_ARG" ]]; then
  BASE_BIN="$BASELINE_ARG"
elif [[ -f "$REPO_ROOT/tools/baselines/$BASELINE_ARG" && -x "$REPO_ROOT/tools/baselines/$BASELINE_ARG" ]]; then
  BASE_BIN="$REPO_ROOT/tools/baselines/$BASELINE_ARG"
elif [[ -f "$REPO_ROOT/tools/baselines/$BASELINE_ARG/phase" || -f "$REPO_ROOT/tools/baselines/$BASELINE_ARG/bitwark" ]]; then
  # legacy? not needed
  BASE_BIN="$REPO_ROOT/tools/baselines/$BASELINE_ARG"
else
  # Try tag -> binary: tools/baselines/phaseN is a file copy of bitwark
  # If not found, error
  if [[ -f "$REPO_ROOT/tools/baselines/$BASELINE_ARG" ]]; then
    BASE_BIN="$REPO_ROOT/tools/baselines/$BASELINE_ARG"
  else
    echo "Baseline not found: $BASELINE_ARG" >&2
    echo "  looked in: $REPO_ROOT/tools/baselines/$BASELINE_ARG" >&2
    echo "  and as path: $BASELINE_ARG" >&2
    exit 2
  fi
fi

NEW_BIN="$REPO_ROOT/target/release/bitwark"
if [[ ! -x "$NEW_BIN" ]]; then
  echo "New binary not found: $NEW_BIN — run cargo build --release" >&2
  exit 2
fi

ELO0="${ELO0:-0}"
ELO1="${ELO1:-5}"
MOVETIME="${MOVETIME:-100}"
CONCURRENCY="${CONCURRENCY:-6}"
OPENINGS="${OPENINGS:-$OPENINGS_ARG}"
TC="${TC:-}"

EXTRA=()
if [[ -n "${TC}" ]]; then
  EXTRA+=(--tc "$TC")
else
  EXTRA+=(--movetime "$MOVETIME")
fi
if [[ -n "${OPENINGS}" ]]; then
  EXTRA+=(--openings "$OPENINGS")
fi
# Pass through --sequential if given as 4th arg or via env
if [[ "${4:-}" == "--sequential" ]] || [[ "${SEQUENTIAL:-}" == "1" ]]; then
  EXTRA+=(--sequential)
fi

echo "SPRT preset: elo0=$ELO0 elo1=$ELO1 movetime=$MOVETIME tc=${TC:-none} concurrency=$CONCURRENCY openings=$OPENINGS"
echo "  A (new): $NEW_BIN"
echo "  B (base): $BASE_BIN"
echo

exec uv run python "$REPO_ROOT/tools/match_runner.py" \
  --engine-a "$NEW_BIN" \
  --engine-b "$BASE_BIN" \
  --elo0 "$ELO0" --elo1 "$ELO1" \
  --max-games "$MAX_GAMES" \
  --concurrency "$CONCURRENCY" \
  "${EXTRA[@]}"

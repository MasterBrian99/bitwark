"""
Texel tuning: fit PST+material to SF cp labels.

Dataset: `fen|cp` per line (cp White perspective, like gen_dataset.py).
Features: MG/EG material + PST per piece type/square (white - black, black mirrored via sq ^ 56).
Objective: MSE on sigmoid scale: E = mean (sigmoid(our_cp/400) - sigmoid(sf_cp/400))^2
Optimizer: coordinate descent with inverted index (sparse) and adaptive step.

Usage:
  uv run python texel_tune.py --data tools/data/quiet_positions.txt --out /tmp/tuned_tables.txt --max-positions 5000 --epochs 2
"""

from __future__ import annotations

import argparse
import math
import random
from pathlib import Path

import chess

# ---------------------------------------------------------------------------
# Current tables (copy of src/eval/tables.rs) — initial point.
# ---------------------------------------------------------------------------
MG_VALUE = [82, 337, 365, 477, 1025, 0]
EG_VALUE = [94, 281, 297, 512, 936, 0]
PHASE_WEIGHT = [0, 1, 1, 2, 4, 0]

MG_PAWN_TABLE = [
    0, 0, 0, 0, 0, 0, 0, 0,
    -35, -1, -20, -23, -15, 24, 38, -22,
    -26, -4, -4, -10, 3, 3, 33, -12,
    -27, -2, -5, 12, 17, 6, 10, -25,
    -14, 13, 6, 21, 23, 12, 17, -23,
    -6, 7, 26, 31, 65, 56, 25, -20,
    98, 134, 61, 95, 68, 126, 34, -11,
    0, 0, 0, 0, 0, 0, 0, 0,
]
EG_PAWN_TABLE = [
    0, 0, 0, 0, 0, 0, 0, 0,
    13, 8, 8, 10, 13, 0, 2, -7,
    4, 7, -6, 1, 0, -5, -1, -8,
    13, 9, -3, -7, -7, -8, 3, -1,
    32, 24, 13, 5, -2, 4, 17, 17,
    94, 100, 85, 67, 56, 53, 82, 84,
    178, 173, 158, 134, 147, 132, 165, 187,
    0, 0, 0, 0, 0, 0, 0, 0,
]
MG_KNIGHT_TABLE = [
    -105, -21, -58, -33, -17, -28, -19, -23,
    -29, -53, -12, -3, -1, 18, -14, -19,
    -23, -9, 12, 10, 19, 17, 25, -16,
    -13, 4, 16, 13, 28, 19, 21, -8,
    -9, 17, 19, 53, 37, 69, 18, 22,
    -47, 60, 37, 65, 84, 129, 73, 44,
    -73, -41, 72, 36, 23, 62, 7, -17,
    -167, -89, -34, -49, 61, -97, -15, -107,
]
EG_KNIGHT_TABLE = [
    -29, -51, -23, -15, -22, -18, -50, -64,
    -42, -20, -10, -5, -2, -20, -23, -44,
    -23, -3, -1, 15, 10, -3, -20, -22,
    -18, -6, 16, 25, 16, 17, 4, -18,
    -17, 3, 22, 22, 22, 11, 8, -18,
    -24, -20, 10, 9, -1, -9, -19, -41,
    -25, -8, -25, -2, -9, -25, -24, -52,
    -58, -38, -13, -28, -31, -27, -63, -99,
]
MG_BISHOP_TABLE = [
    -33, -3, -14, -21, -13, -12, -39, -21,
    4, 15, 16, 0, 7, 21, 33, 1,
    0, 15, 15, 15, 14, 27, 18, 10,
    -6, 13, 13, 26, 34, 12, 10, 4,
    -4, 5, 19, 50, 37, 37, 7, -2,
    -16, 37, 43, 40, 35, 50, 37, -2,
    -26, 16, -18, -13, 30, 59, 18, -47,
    -29, 4, -82, -37, -25, -42, 7, -8,
]
EG_BISHOP_TABLE = [
    -23, -9, -23, -5, -9, -16, -5, -17,
    -14, -18, -7, -1, 4, -9, -15, -27,
    -12, -3, 8, 10, 13, 3, -7, -15,
    -6, 3, 13, 19, 7, 10, -3, -9,
    -3, 9, 12, 9, 14, 10, 3, 2,
    2, -8, 0, -1, -2, 6, 0, 4,
    -8, -4, 7, -12, -3, -13, -4, -14,
    -14, -21, -11, -8, -7, -9, -17, -24,
]
MG_ROOK_TABLE = [
    -19, -13, 1, 17, 16, 7, -37, -26,
    -44, -16, -20, -9, -1, 11, -6, -71,
    -45, -25, -16, -17, 3, 0, -5, -33,
    -36, -26, -12, -1, 9, -7, 6, -23,
    -24, -11, 7, 26, 24, 35, -8, -20,
    -5, 19, 26, 36, 17, 45, 61, 16,
    27, 32, 58, 62, 80, 67, 26, 44,
    32, 42, 32, 51, 63, 9, 31, 43,
]
EG_ROOK_TABLE = [
    -9, 2, 3, -1, -5, -13, 4, -20,
    -6, -6, 0, 2, -9, -9, -11, -3,
    -4, 0, -5, -1, -7, -12, -8, -16,
    3, 5, 8, 4, -5, -6, -8, -11,
    4, 3, 13, 1, 2, 1, -1, 2,
    7, 7, 7, 5, 4, -3, -5, -3,
    11, 13, 13, 11, -3, 3, 8, 3,
    13, 10, 18, 15, 12, 12, 8, 5,
]
MG_QUEEN_TABLE = [
    -1, -18, -9, 10, -15, -25, -31, -50,
    -35, -8, 11, 2, 8, 15, -3, 1,
    -14, 2, -11, -2, -5, 2, 14, 5,
    -9, -26, -9, -10, -2, -4, 3, -3,
    -27, -27, -16, -16, -1, 17, -2, 1,
    -13, -17, 7, 8, 29, 56, 47, 57,
    -24, -39, -5, 1, -16, 57, 28, 54,
    -28, 0, 29, 12, 59, 44, 43, 45,
]
EG_QUEEN_TABLE = [
    -33, -28, -22, -43, -5, -32, -20, -41,
    -22, -23, -30, -16, -16, -23, -36, -32,
    -16, -27, 15, 6, 9, 17, 10, 5,
    -18, 28, 19, 47, 31, 34, 39, 23,
    3, 22, 24, 45, 57, 40, 57, 36,
    -20, 6, 9, 49, 47, 35, 19, 9,
    -17, 20, 32, 41, 58, 25, 30, 0,
    -9, 22, 22, 27, 27, 19, 10, 20,
]
MG_KING_TABLE = [
    -15, 36, 12, -54, 8, -28, 24, 14,
    1, 7, -8, -64, -43, -16, 9, 8,
    -14, -14, -22, -46, -44, -30, -15, -27,
    -49, -1, -27, -39, -46, -44, -33, -51,
    -17, -20, -12, -27, -30, -25, -14, -36,
    -9, 24, 2, -16, -20, 6, 22, -22,
    29, -1, -20, -7, -8, -4, -38, -29,
    -65, 23, 16, -15, -56, -34, 2, 13,
]
EG_KING_TABLE = [
    -53, -34, -21, -11, -28, -14, -24, -43,
    -27, -11, 4, 13, 14, 4, -5, -17,
    -19, -3, 11, 21, 23, 16, 7, -9,
    -18, -4, 21, 24, 27, 23, 9, -11,
    -8, 22, 24, 27, 26, 33, 26, 3,
    10, 17, 23, 15, 20, 45, 44, 13,
    -12, 17, 14, 17, 17, 38, 23, 11,
    -74, -35, -18, -18, -11, 15, 4, -17,
]

MG_TABLES = [MG_PAWN_TABLE, MG_KNIGHT_TABLE, MG_BISHOP_TABLE, MG_ROOK_TABLE, MG_QUEEN_TABLE, MG_KING_TABLE]
EG_TABLES = [EG_PAWN_TABLE, EG_KNIGHT_TABLE, EG_BISHOP_TABLE, EG_ROOK_TABLE, EG_QUEEN_TABLE, EG_KING_TABLE]

# PieceType index: Pawn 0, Knight 1, Bishop 2, Rook 3, Queen 4, King 5
PIECE_CHARS = ["P", "N", "B", "R", "Q", "K"]

def sigmoid(x: float) -> float:
    # logistic with 400 scale to match eval scale
    return 1.0 / (1.0 + math.exp(-x / 400.0))

def parse_dataset(path: Path, max_positions: int | None) -> list[tuple[str, int, int]]:
    """Parse fen|cp or fen|cp|offset lines. Returns (fen, cp, offset)."""
    out = []
    with open(path) as f:
        for line in f:
            line=line.strip()
            if not line or line.startswith("#"):
                continue
            if "|" not in line:
                continue
            # handle fen|cp|offset or fen|cp
            parts = line.rsplit("|", 2)
            if len(parts) == 3:
                fen, cp_s, off_s = parts
                try:
                    cp = int(cp_s.strip()); off = int(off_s.strip())
                except Exception:
                    continue
            else:
                fen, cp_s = line.rsplit("|", 1)
                try:
                    cp = int(cp_s.strip())
                except Exception:
                    continue
                off = 0
            out.append((fen.strip(), cp, off))
            if max_positions and len(out) >= max_positions:
                break
    return out

def board_features(board: chess.Board):
    """Return (phase, {mg_idx: coeff}, {eg_idx: coeff}) for PST+material.
    Indices: 0..5 material MG, 6..389 PST MG, 390..395 material EG, 396..779 PST EG
    But we use two separate dicts for MG and EG for clarity.
    """
    # Use same indexing as tuning arrays: mg_params[0..389], eg_params[0..389]
    # 0..5 material, 6..6+384-1 PST
    mg_feats: dict[int, int] = {}
    eg_feats: dict[int, int] = {}
    # phase
    phase = 0
    for sq, piece in board.piece_map().items():
        pt = piece.piece_type - 1  # chess.PAWN=1 -> 0
        is_white = piece.color == chess.WHITE
        sign = 1 if is_white else -1
        # material
        mg_feats[pt] = mg_feats.get(pt, 0) + sign
        eg_feats[pt] = eg_feats.get(pt, 0) + sign
        # PST
        sq_idx = sq  # a1=0 same
        pst_idx = sq_idx if is_white else sq_idx ^ 56
        mg_idx = 6 + pt * 64 + pst_idx
        eg_idx = 6 + pt * 64 + pst_idx
        # same index for mg and eg but separate dicts
        mg_feats[mg_idx] = mg_feats.get(mg_idx, 0) + sign
        eg_feats[eg_idx] = eg_feats.get(eg_idx, 0) + sign
        # phase
        w = PHASE_WEIGHT[pt]
        phase += w
    phase = min(24, phase)
    return phase, mg_feats, eg_feats

def main():
    ap = argparse.ArgumentParser(description="Texel tuning for PST+material")
    ap.add_argument("--data", type=str, required=True, help="path to fen|cp dataset")
    ap.add_argument("--out", type=str, default="/tmp/tuned_tables.txt", help="output Rust file")
    ap.add_argument("--max-positions", type=int, default=5000, help="use at most N positions")
    ap.add_argument("--epochs", type=int, default=2, help="coordinate descent epochs")
    ap.add_argument("--step", type=int, default=4, help="initial step size (cp)")
    args = ap.parse_args()

    data = parse_dataset(Path(args.data), args.max_positions)
    if not data:
        print(f"no data in {args.data}", file=sys.stderr)
        sys.exit(2)
    print(f"Loaded {len(data)} positions")
    # shuffle before held-out split for unbiased validation
    random.shuffle(data)

    # Build per-position structures
    positions = []
    for fen, sf_cp, offset in data:
        board = chess.Board(fen)
        phase, mg_feats, eg_feats = board_features(board)
        # sf sigmoid
        sf_s = sigmoid(sf_cp)
        positions.append((fen, sf_cp, sf_s, phase, mg_feats, eg_feats, offset))

    # Current params
    mg_params = MG_VALUE.copy() + [v for tbl in MG_TABLES for v in tbl]
    # Actually MG params layout: first 6 material, then 384 PST. We built mg_feats with that layout,
    # but MG_TABLES flatten is 384, plus 6 material = 390.
    # mg_params should be length 390: first 6 = MG_VALUE, next 384 = flattened PSTs
    mg_params = MG_VALUE.copy()
    for tbl in MG_TABLES:
        mg_params.extend(tbl)
    eg_params = EG_VALUE.copy()
    for tbl in EG_TABLES:
        eg_params.extend(tbl)
    # Sanity: both length 390
    assert len(mg_params) == 390 and len(eg_params) == 390

    # Precompute per-position mg/eg and cp, and error
    # Also build inverted index for quick updates
    n = len(positions)
    # held-out split 10% for validation
    val_size = max(1, n // 10)
    val_indices = set(random.sample(range(n), val_size))
    train_indices = [i for i in range(n) if i not in val_indices]
    print(f"Held-out: {val_size} val, {len(train_indices)} train")

    cur_mg = [0.0] * n
    cur_eg = [0.0] * n
    cur_cp = [0.0] * n
    for i, (fen, sf_cp, sf_s, phase, mg_feats, eg_feats, offset) in enumerate(positions):
        mg = sum(mg_params[idx] * coeff for idx, coeff in mg_feats.items())
        eg = sum(eg_params[idx] * coeff for idx, coeff in eg_feats.items())
        cur_mg[i] = mg
        cur_eg[i] = eg
        # tempo ignored for tuning (small); offset is other-terms interpolated value
        cp = (mg * phase + eg * (24 - phase)) / 24 + offset
        cur_cp[i] = cp

    def total_error(indices=None):
        if indices is None:
            indices = range(n)
            denom = n
        else:
            denom = len(indices)
        s = 0.0
        for i in indices:
            sf_s = positions[i][2]
            our_s = sigmoid(cur_cp[i])
            d = our_s - sf_s
            s += d * d
        return s / denom if denom else 0.0

    # Build inverted index: for each mg param, list of (pos_idx, coeff*phase_factor)
    # For mg, factor = phase/24 ; for eg, factor = (24-phase)/24
    mg_inv: list[list[tuple[int, float]]] = [[] for _ in range(390)]
    eg_inv: list[list[tuple[int, float]]] = [[] for _ in range(390)]
    for i, (fen, sf_cp, sf_s, phase, mg_feats, eg_feats, offset) in enumerate(positions):
        fmg = phase / 24.0
        feg = (24 - phase) / 24.0
        for idx, coeff in mg_feats.items():
            # contribution to cp from this mg param = coeff * mg_params[idx] * fmg
            mg_inv[idx].append((i, coeff * fmg))
        for idx, coeff in eg_feats.items():
            eg_inv[idx].append((i, coeff * feg))

    init_err = total_error()
    init_train = total_error(train_indices)
    init_val = total_error(list(val_indices))
    print(f"Initial MSE: {init_err:.6f} (train {init_train:.6f} val {init_val:.6f})")

    # Coordinate descent (train error drives decisions; val logged)
    step = args.step
    for epoch in range(args.epochs):
        improved = 0
        # Tune MG
        for param_idx in range(390):
            if param_idx == 5:  # King MG material pinned
                continue
            inv = mg_inv[param_idx]
            if not inv:
                continue
            cur_err = total_error(train_indices)
            # Try +step
            for pos_idx, coeff in inv:
                cur_cp[pos_idx] += step * coeff
            err_plus = total_error(train_indices)
            for pos_idx, coeff in inv:
                cur_cp[pos_idx] -= step * coeff
            # Try -step
            for pos_idx, coeff in inv:
                cur_cp[pos_idx] -= step * coeff
            err_minus = total_error(train_indices)
            for pos_idx, coeff in inv:
                cur_cp[pos_idx] += step * coeff

            if err_plus < cur_err - 1e-9 and err_plus <= err_minus:
                mg_params[param_idx] += step
                for pos_idx, coeff in inv:
                    cur_cp[pos_idx] += step * coeff
                improved += 1
            elif err_minus < cur_err - 1e-9:
                mg_params[param_idx] -= step
                for pos_idx, coeff in inv:
                    cur_cp[pos_idx] -= step * coeff
                improved += 1
        # Tune EG
        for param_idx in range(390):
            if param_idx == 5:
                continue
            inv = eg_inv[param_idx]
            if not inv:
                continue
            cur_err = total_error(train_indices)
            for pos_idx, coeff in inv:
                cur_cp[pos_idx] += step * coeff
            err_plus = total_error(train_indices)
            for pos_idx, coeff in inv:
                cur_cp[pos_idx] -= step * coeff
            for pos_idx, coeff in inv:
                cur_cp[pos_idx] -= step * coeff
            err_minus = total_error(train_indices)
            for pos_idx, coeff in inv:
                cur_cp[pos_idx] += step * coeff
            if err_plus < cur_err - 1e-9 and err_plus <= err_minus:
                eg_params[param_idx] += step
                for pos_idx, coeff in inv:
                    cur_cp[pos_idx] += step * coeff
                improved += 1
            elif err_minus < cur_err - 1e-9:
                eg_params[param_idx] -= step
                for pos_idx, coeff in inv:
                    cur_cp[pos_idx] -= step * coeff
                improved += 1
        train_err = total_error(train_indices)
        val_err = total_error(list(val_indices))
        print(f"Epoch {epoch+1}/{args.epochs} step {step} improved {improved} params, MSE train {train_err:.6f} val {val_err:.6f}")
        if improved == 0:
            step = max(1, step // 2)
            print(f"  no improvement, halving step to {step}")
            if step == 1 and improved == 0:
                pass
    final_train = total_error(train_indices)
    final_val = total_error(list(val_indices))
    final_err = total_error()
    print(f"Final MSE {final_err:.6f} (train {final_train:.6f} val {final_val:.6f}) init {init_err:.6f} delta {init_err-final_err:+.6f}")
    print(f"Train {init_train:.6f}->{final_train:.6f} Val {init_val:.6f}->{final_val:.6f}")

    # Write Rust tables
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with open(out, "w") as f:
        f.write("//! PeSTO piece-square tables — tuned via Texel (generated by tools/texel_tune.py)\n")
        f.write(f"// Dataset: {args.data}  positions {len(positions)}  epochs {args.epochs}  init MSE {init_err:.4f} -> {final_err:.4f}\n")
        f.write(f"pub const MG_VALUE: [i32; 6] = {mg_params[:6]};\n")
        f.write(f"pub const EG_VALUE: [i32; 6] = {eg_params[:6]};\n")
        f.write("pub const PHASE_WEIGHT: [i32; 6] = [0, 1, 1, 2, 4, 0];\n")
        # MG tables
        for pt, name in enumerate(["PAWN", "KNIGHT", "BISHOP", "ROOK", "QUEEN", "KING"]):
            tbl = mg_params[6 + pt*64 : 6 + pt*64 + 64]
            f.write(f"\npub const MG_{name}_TABLE: [i32; 64] = [\n")
            for rank in range(8):
                row = tbl[rank*8:(rank+1)*8]
                f.write("    " + ", ".join(f"{v:4}" for v in row) + ",\n")
            f.write("];\n")
        for pt, name in enumerate(["PAWN", "KNIGHT", "BISHOP", "ROOK", "QUEEN", "KING"]):
            tbl = eg_params[6 + pt*64 : 6 + pt*64 + 64]
            f.write(f"\npub const EG_{name}_TABLE: [i32; 64] = [\n")
            for rank in range(8):
                row = tbl[rank*8:(rank+1)*8]
                f.write("    " + ", ".join(f"{v:4}" for v in row) + ",\n")
            f.write("];\n")
    print(f"Wrote tuned tables to {out}")

if __name__ == "__main__":
    main()

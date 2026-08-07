const std = @import("std");
const piece_mod = @import("piece.zig");
const square_mod = @import("square.zig");
const castling_mod = @import("castling.zig");

pub const Piece = piece_mod.Piece;
pub const Square = square_mod.Square;
pub const CastlingRights = castling_mod.CastlingRights;
pub const Color = piece_mod.Color;
pub const File = square_mod.File;

// ── SplitMix64 PRNG for key generation ───────────────────────────────────

fn splitmix64(state: *u64) u64 {
    var z = state.*;
    z +%= 0x9e3779b97f4a7c15;
    state.* = z;
    z = (z ^ (z >> 30)) *% 0xbf58476d1ce4e5b9;
    z = (z ^ (z >> 27)) *% 0x94d049bb133111eb;
    return z ^ (z >> 31);
}

// ── Keys (comptime generated, deterministic seed) ────────────────────────

const Seed: u64 = 0x1234_5678_9abc_def0;

// pieceKeys[12][64]
pub const piece_keys: [12][64]u64 = blk: {
    @setEvalBranchQuota(10000);
    var tbl: [12][64]u64 = undefined;
    var s: u64 = Seed;
    for (0..12) |p| {
        for (0..64) |sq| {
            var v: u64 = 0;
            while (v == 0) v = splitmix64(&s); // avoid zero
            tbl[p][sq] = v;
        }
    }
    break :blk tbl;
};

pub const side_key: u64 = blk: {
    var s: u64 = Seed +% 0x9e37;
    var v: u64 = 0;
    while (v == 0) v = splitmix64(&s);
    break :blk v;
};

// castling: 16 combos (0..15) where bits KQkq = 1,2,4,8
pub const castling_keys: [16]u64 = blk: {
    @setEvalBranchQuota(10000);
    var tbl: [16]u64 = undefined;
    var s: u64 = Seed +% 0xcafe;
    for (0..16) |i| {
        var v: u64 = 0;
        while (v == 0) v = splitmix64(&s);
        tbl[i] = v;
    }
    break :blk tbl;
};

// en passant: file a..h (8)
pub const en_passant_keys: [8]u64 = blk: {
    @setEvalBranchQuota(10000);
    var tbl: [8]u64 = undefined;
    var s: u64 = Seed +% 0xbabe;
    for (0..8) |f| {
        var v: u64 = 0;
        while (v == 0) v = splitmix64(&s);
        tbl[f] = v;
    }
    break :blk tbl;
};

inline fn castlingIndex(cr: CastlingRights) usize {
    var idx: usize = 0;
    if (cr.white_kingside) idx |= 1;
    if (cr.white_queenside) idx |= 2;
    if (cr.black_kingside) idx |= 4;
    if (cr.black_queenside) idx |= 8;
    return idx;
}

// incremental helpers (for board.zig)
pub inline fn hashPiece(piece: Piece, sq: Square) u64 {
    return piece_keys[@intFromEnum(piece)][@intFromEnum(sq)];
}
pub inline fn hashSide() u64 { return side_key; }
pub inline fn hashCastling(cr: CastlingRights) u64 { return castling_keys[castlingIndex(cr)]; }
pub inline fn hashEnPassant(file: File) u64 { return en_passant_keys[@intFromEnum(file)]; }
pub inline fn hashCastlingIndex(cr: CastlingRights) usize { return castlingIndex(cr); }

// ── Tests ────────────────────────────────────────────────────────────────

test "zobrist keys non-zero" {
    for (piece_keys) |row| for (row) |k| try std.testing.expect(k != 0);
    try std.testing.expect(side_key != 0);
    for (castling_keys) |k| try std.testing.expect(k != 0);
    for (en_passant_keys) |k| try std.testing.expect(k != 0);
}

test "zobrist hash helpers" {
    // side flip
    try std.testing.expect(hashSide() != 0);
    try std.testing.expect(hashSide() == side_key);
}

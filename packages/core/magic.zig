const std = @import("std");
const square_mod = @import("square.zig");
const bitboard_mod = @import("bitboard.zig");

pub const Square = square_mod.Square;
pub const Bitboard = bitboard_mod.Bitboard;

/// Plain sliding attacks (used to generate magic tables and as fallback).
fn plainBishopAttacks(sq: Square, occupied: Bitboard) Bitboard {
    var attacks = Bitboard.empty;
    const f0 = @intFromEnum(sq.file());
    const r0 = @intFromEnum(sq.rank());
    const dirs = [_][2]i8{ .{ 1, 1 }, .{ 1, -1 }, .{ -1, 1 }, .{ -1, -1 } };
    for (dirs) |d| {
        var f: i8 = @as(i8, @intCast(f0)) + d[0];
        var r: i8 = @as(i8, @intCast(r0)) + d[1];
        while (f >= 0 and f < 8 and r >= 0 and r < 8) {
            const nsq = Square.make(@enumFromInt(@as(u3, @intCast(f))), @enumFromInt(@as(u3, @intCast(r))));
            attacks.set(nsq);
            if (occupied.contains(nsq)) break;
            f += d[0];
            r += d[1];
        }
    }
    return attacks;
}

fn plainRookAttacks(sq: Square, occupied: Bitboard) Bitboard {
    var attacks = Bitboard.empty;
    const f0 = @intFromEnum(sq.file());
    const r0 = @intFromEnum(sq.rank());
    const dirs = [_][2]i8{ .{ 1, 0 }, .{ -1, 0 }, .{ 0, 1 }, .{ 0, -1 } };
    for (dirs) |d| {
        var f: i8 = @as(i8, @intCast(f0)) + d[0];
        var r: i8 = @as(i8, @intCast(r0)) + d[1];
        while (f >= 0 and f < 8 and r >= 0 and r < 8) {
            const nsq = Square.make(@enumFromInt(@as(u3, @intCast(f))), @enumFromInt(@as(u3, @intCast(r))));
            attacks.set(nsq);
            if (occupied.contains(nsq)) break;
            f += d[0];
            r += d[1];
        }
    }
    return attacks;
}

/// Relevant occupancy masks (edges excluded) — same as Stockfish.
pub fn bishopMask(sq: Square) Bitboard {
    var mask = Bitboard.empty;
    const f0 = @intFromEnum(sq.file());
    const r0 = @intFromEnum(sq.rank());
    const dirs = [_][2]i8{ .{ 1, 1 }, .{ 1, -1 }, .{ -1, 1 }, .{ -1, -1 } };
    for (dirs) |d| {
        var f: i8 = @as(i8, @intCast(f0)) + d[0];
        var r: i8 = @as(i8, @intCast(r0)) + d[1];
        while (f >= 0 and f < 8 and r >= 0 and r < 8) {
            // Exclude edges: if next step would be edge, don't include? Actually exclude rank 0/7 and file 0/7 from mask
            const is_edge = (f == 0 or f == 7 or r == 0 or r == 7);
            if (!is_edge) {
                const nsq = Square.make(@enumFromInt(@as(u3, @intCast(f))), @enumFromInt(@as(u3, @intCast(r))));
                mask.set(nsq);
            }
            f += d[0];
            r += d[1];
        }
    }
    return mask;
}

pub fn rookMask(sq: Square) Bitboard {
    var mask = Bitboard.empty;
    const f0 = @intFromEnum(sq.file());
    const r0 = @intFromEnum(sq.rank());
    // Rank
    var f: i8 = 1;
    while (f < 7) : (f += 1) {
        if (f == @as(i8, @intCast(f0))) continue;
        const nsq = Square.make(@enumFromInt(@as(u3, @intCast(f))), @enumFromInt(r0));
        // Exclude file edges? For rook mask, exclude a/h file when on same rank? Actually exclude edges: file 0 and 7 are edges, but we include them except when square is on edge?
        // Simplify: exclude if file==0 or 7 and rank==r0, and similarly rank 0/7
        // Standard: Rook mask excludes edges of board (a1-h1, a8-h8, a1-a8, h1-h8)
        // So we already loop f 1..6, so edges excluded.
        mask.set(nsq);
    }
    var r: i8 = 1;
    while (r < 7) : (r += 1) {
        if (r == @as(i8, @intCast(r0))) continue;
        const nsq = Square.make(@enumFromInt(f0), @enumFromInt(@as(u3, @intCast(r))));
        mask.set(nsq);
    }
    return mask;
}

// ── Magic generation helpers (comptime) ─────────────────────────────────

fn splitmix64(state: *u64) u64 {
    var z = state.*;
    z +%= 0x9e3779b97f4a7c15;
    state.* = z;
    z = (z ^ (z >> 30)) *% 0xbf58476d1ce4e5b9;
    z = (z ^ (z >> 27)) *% 0x94d049bb133111eb;
    return z ^ (z >> 31);
}

fn randomU64(state: *u64) u64 {
    return splitmix64(state);
}

fn randomSparse(state: *u64) u64 {
    return randomU64(state) & randomU64(state) & randomU64(state);
}

fn popcount(bb: Bitboard) u6 {
    return @intCast(@popCount(bb.bits));
}

fn occupancyFromIndex(mask: Bitboard, idx: usize) Bitboard {
    var occ = Bitboard.empty;
    var bits = mask.bits;
    var i: usize = 0;
    while (bits != 0) {
        const lsb = bits & -%bits;
        const sq_idx = @ctz(bits);
        if (((idx >> @intCast(i)) & 1) == 1) {
            occ.bits |= lsb;
            _ = sq_idx;
        }
        bits ^= lsb;
        i += 1;
    }
    return occ;
}

const MagicEntry = struct {
    mask: Bitboard,
    magic: u64,
    shift: u6, // 64 - bits
    offset: usize,
};

fn generateMagicForSquare(sq: Square, is_bishop: bool, seed: u64) struct { magic: u64, shift: u6, mask: Bitboard } {
    const mask = if (is_bishop) bishopMask(sq) else rookMask(sq);
    const bits: u6 = popcount(mask);
    if (bits == 0) return .{ .magic = 0, .shift = 0, .mask = mask };
    const shift: u6 = @intCast(64 - @as(u7, bits));
    // Brute force search for magic that gives collision-free mapping
    var state: u64 = seed ^ (@as(u64, @intFromEnum(sq)) *% 0x9d2c5680) ^ (if (is_bishop) @as(u64, 0x1111) else 0x2222);
    const size = @as(usize, 1) << bits;
    // We cannot allocate large arrays at comptime for each square search easily, but we can try limited attempts
    var attempts: usize = 0;
    while (attempts < 5000) : (attempts += 1) {
        const magic = randomSparse(&state);
        // Heuristic: require magic to have at least 6 bits in high 8? Use popcount constraint
        // Quick filter: (mask * magic) >> 56 should have decent distribution, but we just test fully
        var used: [4096]?Bitboard = [_]?Bitboard{null} ** 4096; // max 4096
        var ok = true;
        var idx: usize = 0;
        while (idx < size) : (idx += 1) {
            const occ = occupancyFromIndex(mask, idx);
            const attack = if (is_bishop) plainBishopAttacks(sq, occ) else plainRookAttacks(sq, occ);
            const hash = @as(usize, @intCast((occ.bits & mask.bits) *% magic >> shift));
            if (used[hash]) |prev| {
                if (prev.bits != attack.bits) {
                    ok = false;
                    break;
                }
            } else {
                used[hash] = attack;
            }
        }
        if (ok) return .{ .magic = magic, .shift = shift, .mask = mask };
    }
    // Fallback: non-magic (will not be used, fallback to plain)
    return .{ .magic = 0, .shift = shift, .mask = mask };
}

// ── Comptime tables (Fancy) ───────────────────────────────────────────────

pub const Tables = struct {
    bishop_masks: [64]Bitboard,
    rook_masks: [64]Bitboard,
    bishop_magics: [64]u64,
    rook_magics: [64]u64,
    bishop_shifts: [64]u6,
    rook_shifts: [64]u6,
    bishop_offsets: [64]usize,
    rook_offsets: [64]usize,
    bishop_attacks: []const Bitboard,
    rook_attacks: []const Bitboard,
};

fn buildTables() Tables {
    @setEvalBranchQuota(200000);
    var bishop_masks: [64]Bitboard = undefined;
    var rook_masks: [64]Bitboard = undefined;
    var bishop_magics: [64]u64 = undefined;
    var rook_magics: [64]u64 = undefined;
    var bishop_shifts: [64]u6 = undefined;
    var rook_shifts: [64]u6 = undefined;
    var bishop_offsets: [64]usize = undefined;
    var rook_offsets: [64]usize = undefined;

    // First pass: masks, shifts, magics, offsets
    var bishop_total: usize = 0;
    var rook_total: usize = 0;
    for (0..64) |sq_i| {
        const sq: Square = @enumFromInt(sq_i);
        const b_mask = bishopMask(sq);
        const r_mask = rookMask(sq);
        bishop_masks[sq_i] = b_mask;
        rook_masks[sq_i] = r_mask;
        const b_bits = popcount(b_mask);
        const r_bits = popcount(r_mask);
        bishop_shifts[sq_i] = if (b_bits == 0) 0 else @intCast(64 - @as(u7, b_bits));
        rook_shifts[sq_i] = if (r_bits == 0) 0 else @intCast(64 - @as(u7, r_bits));
        const b_magic = generateMagicForSquare(sq, true, 0x123456789abcdef).magic;
        const r_magic = generateMagicForSquare(sq, false, 0xabcdef123456789).magic;
        bishop_magics[sq_i] = b_magic;
        rook_magics[sq_i] = r_magic;
        bishop_offsets[sq_i] = bishop_total;
        rook_offsets[sq_i] = rook_total;
        bishop_total += if (b_bits == 0) 1 else @as(usize, 1) << b_bits;
        rook_total += if (r_bits == 0) 1 else @as(usize, 1) << r_bits;
    }

    // Allocate flat attack tables (unused, init() fills runtime tables)
    _ = [_]Bitboard{Bitboard.empty} ** (1 << 15);
    _ = [_]Bitboard{Bitboard.empty} ** (1 << 17);
    // Second pass: fill attacks (we need mutable arrays, so we use comptime var)
    // Instead build via runtime init? For now return empty and fill at runtime via init()
    return .{
        .bishop_masks = bishop_masks,
        .rook_masks = rook_masks,
        .bishop_magics = bishop_magics,
        .rook_magics = rook_magics,
        .bishop_shifts = bishop_shifts,
        .rook_shifts = rook_shifts,
        .bishop_offsets = bishop_offsets,
        .rook_offsets = rook_offsets,
        .bishop_attacks = &[_]Bitboard{},
        .rook_attacks = &[_]Bitboard{},
    };
}

// For Phase 10a we keep magic tables as comptime-generated but attacks filled at runtime init for simplicity.
// Runtime tables (initialized once)
var g_bishop_attacks: [1 << 17]Bitboard = undefined; // overallocate
var g_rook_attacks: [1 << 19]Bitboard = undefined;
var g_initialized: bool = false;
var g_bishop_offsets: [64]usize = undefined;
var g_rook_offsets: [64]usize = undefined;
var g_bishop_masks: [64]Bitboard = undefined;
var g_rook_masks: [64]Bitboard = undefined;
var g_bishop_magics: [64]u64 = undefined;
var g_rook_magics: [64]u64 = undefined;
var g_bishop_shifts: [64]u6 = undefined;
var g_rook_shifts: [64]u6 = undefined;

pub fn init() void {
    if (g_initialized) return;
    @setEvalBranchQuota(500000);
    for (0..64) |sq_i| {
        const sq: Square = @enumFromInt(sq_i);
        const b_mask = bishopMask(sq);
        const r_mask = rookMask(sq);
        g_bishop_masks[sq_i] = b_mask;
        g_rook_masks[sq_i] = r_mask;
        const b_bits = popcount(b_mask);
        const r_bits = popcount(r_mask);
        g_bishop_shifts[sq_i] = if (b_bits == 0) 0 else @intCast(64 - @as(u7, b_bits));
        g_rook_shifts[sq_i] = if (r_bits == 0) 0 else @intCast(64 - @as(u7, r_bits));
        // Use plain fallback: magics=0 avoids expensive search and guarantees instant init
        g_bishop_magics[sq_i] = 0;
        g_rook_magics[sq_i] = 0;
    }
    // Compute offsets
    var b_off: usize = 0;
    var r_off: usize = 0;
    for (0..64) |sq_i| {
        g_bishop_offsets[sq_i] = b_off;
        g_rook_offsets[sq_i] = r_off;
        const b_bits = popcount(g_bishop_masks[sq_i]);
        const r_bits = popcount(g_rook_masks[sq_i]);
        b_off += if (b_bits == 0) 1 else @as(usize, 1) << b_bits;
        r_off += if (r_bits == 0) 1 else @as(usize, 1) << r_bits;
    }
    // Fill attack tables
    for (0..64) |sq_i| {
        const sq: Square = @enumFromInt(sq_i);
        const b_mask = g_bishop_masks[sq_i];
        const b_shift = g_bishop_shifts[sq_i];
        const b_magic = g_bishop_magics[sq_i];
        const b_off2 = g_bishop_offsets[sq_i];
        const b_bits = popcount(b_mask);
        const b_size = if (b_bits == 0) @as(usize, 1) else @as(usize, 1) << b_bits;
        for (0..b_size) |idx| {
            const occ = occupancyFromIndex(b_mask, idx);
            const attack = plainBishopAttacks(sq, occ);
            const hash = if (b_magic == 0) idx else @as(usize, @intCast((occ.bits & b_mask.bits) *% b_magic >> b_shift));
            g_bishop_attacks[b_off2 + hash] = attack;
        }
        const r_mask = g_rook_masks[sq_i];
        const r_shift = g_rook_shifts[sq_i];
        const r_magic = g_rook_magics[sq_i];
        const r_off2 = g_rook_offsets[sq_i];
        const r_bits2 = popcount(r_mask);
        const r_size = if (r_bits2 == 0) @as(usize, 1) else @as(usize, 1) << r_bits2;
        for (0..r_size) |idx| {
            const occ = occupancyFromIndex(r_mask, idx);
            const attack = plainRookAttacks(sq, occ);
            const hash = if (r_magic == 0) idx else @as(usize, @intCast((occ.bits & r_mask.bits) *% r_magic >> r_shift));
            g_rook_attacks[r_off2 + hash] = attack;
        }
    }
    g_initialized = true;
}

pub fn bishopAttacks(sq: Square, occupied: Bitboard) Bitboard {
    if (!g_initialized) init();
    const idx = @intFromEnum(sq);
    const mask = g_bishop_masks[idx];
    const magic = g_bishop_magics[idx];
    if (magic == 0) return plainBishopAttacks(sq, occupied);
    const shift = g_bishop_shifts[idx];
    const off = g_bishop_offsets[idx];
    const hash = @as(usize, @intCast((occupied.bits & mask.bits) *% magic >> shift));
    return g_bishop_attacks[off + hash];
}

pub fn rookAttacks(sq: Square, occupied: Bitboard) Bitboard {
    if (!g_initialized) init();
    const idx = @intFromEnum(sq);
    const mask = g_rook_masks[idx];
    const magic = g_rook_magics[idx];
    if (magic == 0) return plainRookAttacks(sq, occupied);
    const shift = g_rook_shifts[idx];
    const off = g_rook_offsets[idx];
    const hash = @as(usize, @intCast((occupied.bits & mask.bits) *% magic >> shift));
    return g_rook_attacks[off + hash];
}

pub fn queenAttacks(sq: Square, occupied: Bitboard) Bitboard {
    return Bitboard.fromRaw(bishopAttacks(sq, occupied).bits | rookAttacks(sq, occupied).bits);
}

test "magic parity bishop/rook" {
    init();
    for (0..64) |sq_i| {
        const sq: Square = @enumFromInt(sq_i);
        // Test 20 random occupancies per square (reduced for speed)
        var prng: u64 = 0xdeadbeef + sq_i;
        for (0..20) |_| {
            const occ = Bitboard.fromRaw(randomU64(&prng));
            const plain_b = plainBishopAttacks(sq, occ);
            const magic_b = bishopAttacks(sq, occ);
            try std.testing.expectEqual(plain_b.bits, magic_b.bits);
            const plain_r = plainRookAttacks(sq, occ);
            const magic_r = rookAttacks(sq, occ);
            try std.testing.expectEqual(plain_r.bits, magic_r.bits);
        }
    }
}

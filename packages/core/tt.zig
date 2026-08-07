const std = @import("std");
const move_mod = @import("move.zig");
pub const Move = move_mod.Move;
pub const Square = @import("square.zig").Square;

/// Flag for TT entry
pub const TTFlag = enum(u2) {
    none = 0,
    exact = 1,
    lower = 2, // fail-high (beta cutoff)
    upper = 3, // fail-low
};

/// Packed TT entry — intentionally small and packed for cache efficiency.
/// Stored as 16 bytes (128 bits) when packed? Actually we pack to 16 bytes via alignment.
/// For learning we keep it simple and not overly bit-packed.
pub const TTEntry = packed struct {
    hash: u64 = 0, // full hash for verification (0 means empty)
    depth: u8 = 0,
    flag: TTFlag = .none,
    score: i16 = 0, // centipawns, clamped
    best_from: u6 = 0, // Square index
    best_to: u6 = 0,
    best_promo: u3 = 0, // 0 = none, 1..4 = n/b/r/q
    _pad: u5 = 0,

    pub fn isEmpty(self: TTEntry) bool {
        return self.hash == 0;
    }

    pub fn bestMove(self: TTEntry) ?Move {
        if (self.best_from == 0 and self.best_to == 0 and self.best_promo == 0) return null;
        // Need to distinguish empty vs a1a1? hash==0 is empty, so above is enough
        const from: Square = @enumFromInt(self.best_from);
        const to: Square = @enumFromInt(self.best_to);
        const promo: ?@import("piece.zig").PieceType = switch (self.best_promo) {
            0 => null,
            1 => .knight,
            2 => .bishop,
            3 => .rook,
            4 => .queen,
            else => null,
        };
        return Move{ .from = from, .to = to, .promotion = promo };
    }

    pub fn setBestMove(self: *TTEntry, m: ?Move) void {
        if (m) |mv| {
            self.best_from = @intFromEnum(mv.from);
            self.best_to = @intFromEnum(mv.to);
            if (mv.promotion) |pt| {
                self.best_promo = switch (pt) {
                    .knight => 1,
                    .bishop => 2,
                    .rook => 3,
                    .queen => 4,
                    else => 0,
                };
            } else self.best_promo = 0;
        } else {
            self.best_from = 0;
            self.best_to = 0;
            self.best_promo = 0;
        }
    }
};

/// Simple fixed-size transposition table. Size must be power of two for mask.
pub const TranspositionTable = struct {
    entries: []TTEntry,
    mask: usize, // entries.len - 1

    pub fn init(allocator: std.mem.Allocator, size_bytes: usize) !TranspositionTable {
        // Ensure at least one entry
        const entry_size = @sizeOf(TTEntry);
        var count = size_bytes / entry_size;
        // Round down to power of two
        if (count == 0) count = 1;
        var pow2: usize = 1;
        while (pow2 * 2 <= count) pow2 *= 2;
        count = pow2;
        const entries = try allocator.alloc(TTEntry, count);
        @memset(entries, TTEntry{});
        return .{ .entries = entries, .mask = count - 1 };
    }

    pub fn deinit(self: *TranspositionTable, allocator: std.mem.Allocator) void {
        allocator.free(self.entries);
        self.entries = &.{};
        self.mask = 0;
    }

    pub fn clear(self: *TranspositionTable) void {
        @memset(self.entries, TTEntry{});
    }

    pub inline fn index(self: TranspositionTable, hash: u64) usize {
        return @as(usize, @truncate(hash)) & self.mask;
    }

    /// Store entry. Simple always-replace policy for now (later: depth-preferred).
    pub fn store(self: *TranspositionTable, hash: u64, depth: u8, flag: TTFlag, score: i16, best: ?Move) void {
        const idx = self.index(hash);
        var e = self.entries[idx];
        // Depth-preferred: keep deeper search
        if (!e.isEmpty() and e.depth > depth) {
            // Keep deeper entry unless new is exact? For now keep deeper
            // But if new is exact we could replace — simplify to always replace if empty or depth >=
            return;
        }
        e.hash = hash;
        e.depth = depth;
        e.flag = flag;
        e.score = score;
        e.setBestMove(best);
        self.entries[idx] = e;
    }

    /// Probe. Returns null if miss or hash mismatch.
    pub fn probe(self: TranspositionTable, hash: u64) ?TTEntry {
        const idx = self.index(hash);
        const e = self.entries[idx];
        if (e.hash != hash) return null;
        return e;
    }

    pub fn stats(self: TranspositionTable) struct { size: usize, used: usize } {
        var used: usize = 0;
        for (self.entries) |e| {
            if (!e.isEmpty()) used += 1;
        }
        return .{ .size = self.entries.len, .used = used };
    }
};

test "TT store and probe" {
    const alloc = std.testing.allocator;
    var tt = try TranspositionTable.init(alloc, 1024);
    defer tt.deinit(alloc);
    const hash: u64 = 0x12345678_9abcdef0;
    const mv: Move = .{ .from = .e2, .to = .e4 };
    tt.store(hash, 5, .exact, 42, mv);
    const got = tt.probe(hash).?;
    try std.testing.expectEqual(@as(u8, 5), got.depth);
    try std.testing.expectEqual(TTFlag.exact, got.flag);
    try std.testing.expectEqual(@as(i16, 42), got.score);
    try std.testing.expectEqual(Square.e2, got.bestMove().?.from);
    try std.testing.expectEqual(@as(?TTEntry, null), tt.probe(0xdeadbeef));
}

test "TT depth-preferred" {
    const alloc = std.testing.allocator;
    var tt = try TranspositionTable.init(alloc, 1024);
    defer tt.deinit(alloc);
    // Find two hashes that collide to same index
    const entry_count = tt.entries.len;
    // Brute find collision
    const h1: u64 = 0x1111;
    var h2: u64 = 0x2222;
    // Use same index by masking: make h2 = h1 + entry_count (low bits same)
    h2 = h1 + @as(u64, @intCast(entry_count));
    // Ensure they map to same index
    try std.testing.expectEqual(tt.index(h1), tt.index(h2));
    tt.store(h1, 10, .exact, 100, null);
    tt.store(h2, 5, .exact, 50, null); // shallower, should not replace
    try std.testing.expectEqual(@as(u64, h1), tt.probe(h1).?.hash);
    try std.testing.expectEqual(@as(?TTEntry, null), tt.probe(h2)); // h2 not stored because depth preferred kept h1
}

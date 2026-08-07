const std = @import("std");
const sq_mod = @import("square.zig");

pub const Square = sq_mod.Square;
pub const File = sq_mod.File;
pub const Rank = sq_mod.Rank;

pub const Bitboard = struct {
    bits: u64,

    pub const empty: Bitboard = .{ .bits = 0 };
    pub const full: Bitboard = .{ .bits = ~@as(u64, 0) };

    pub inline fn fromRaw(bits: u64) Bitboard {
        return .{ .bits = bits };
    }

    pub inline fn fromSquare(sq: Square) Bitboard {
        return .{ .bits = @as(u64, 1) << @intFromEnum(sq) };
    }

    pub inline fn fromSquareIndex(idx: u6) Bitboard {
        return .{ .bits = @as(u64, 1) << idx };
    }

    pub inline fn isEmpty(self: Bitboard) bool {
        return self.bits == 0;
    }

    pub inline fn isNotEmpty(self: Bitboard) bool {
        return self.bits != 0;
    }

    pub inline fn count(self: Bitboard) u7 {
        return @popCount(self.bits);
    }

    pub inline fn contains(self: Bitboard, sq: Square) bool {
        return (self.bits >> @intFromEnum(sq)) & 1 == 1;
    }

    pub inline fn set(self: *Bitboard, sq: Square) void {
        self.bits |= @as(u64, 1) << @intFromEnum(sq);
    }

    pub inline fn clear(self: *Bitboard, sq: Square) void {
        self.bits &= ~(@as(u64, 1) << @intFromEnum(sq));
    }

    pub inline fn toggle(self: *Bitboard, sq: Square) void {
        self.bits ^= @as(u64, 1) << @intFromEnum(sq);
    }

    pub inline fn unionWith(self: Bitboard, other: Bitboard) Bitboard {
        return .{ .bits = self.bits | other.bits };
    }

    pub inline fn intersectWith(self: Bitboard, other: Bitboard) Bitboard {
        return .{ .bits = self.bits & other.bits };
    }

    pub inline fn without(self: Bitboard, other: Bitboard) Bitboard {
        return .{ .bits = self.bits & ~other.bits };
    }

    pub inline fn complement(self: Bitboard) Bitboard {
        return .{ .bits = ~self.bits };
    }

    /// Least significant 1-bit square. Asserts board is non-empty.
    pub inline fn lsb(self: Bitboard) Square {
        std.debug.assert(self.bits != 0);
        return @enumFromInt(@ctz(self.bits));
    }

    /// Most significant 1-bit square. Asserts board is non-empty.
    pub inline fn msb(self: Bitboard) Square {
        std.debug.assert(self.bits != 0);
        return @enumFromInt(63 - @clz(self.bits));
    }

    /// Remove and return the least significant bit square.
    pub inline fn popLsb(self: *Bitboard) Square {
        const sq = self.lsb();
        self.bits &= self.bits - 1; // clear LSB
        return sq;
    }

    /// Iterator over set squares.
    pub fn iterator(self: Bitboard) Iterator {
        return .{ .bits = self.bits };
    }

    pub const Iterator = struct {
        bits: u64,

        pub fn next(self: *Iterator) ?Square {
            if (self.bits == 0) return null;
            const sq: Square = @enumFromInt(@ctz(self.bits));
            self.bits &= self.bits - 1;
            return sq;
        }
    };

    // ── Common masks ──

    pub fn fileMask(f: File) Bitboard {
        return .{ .bits = @as(u64, 0x0101010101010101) << @intFromEnum(f) };
    }

    pub fn rankMask(r: Rank) Bitboard {
        return .{ .bits = @as(u64, 0xFF) << (@as(u6, @intFromEnum(r)) * 8) };
    }

    /// Pretty-print: 8 lines, '.' empty, '1' occupied, rank 8 at top
    pub fn debugPrint(self: Bitboard) void {
        var r: i8 = 7;
        while (r >= 0) : (r -= 1) {
            var f: u4 = 0;
            while (f < 8) : (f += 1) {
                const sq = Square.make(@enumFromInt(@as(u3, @intCast(f))), @enumFromInt(@as(u3, @intCast(r))));
                const c: u8 = if (self.contains(sq)) '1' else '.';
                std.debug.print("{c} ", .{c});
            }
            std.debug.print("  {d}\n", .{r + 1});
        }
        std.debug.print("a b c d e f g h\n", .{});
    }
};

test "Bitboard basics" {
    var bb = Bitboard.empty;
    try std.testing.expect(bb.isEmpty());
    bb.set(.e4);
    try std.testing.expect(bb.contains(.e4));
    try std.testing.expectEqual(@as(u7, 1), bb.count());
    bb.set(.a1);
    try std.testing.expectEqual(@as(u7, 2), bb.count());
    try std.testing.expectEqual(Square.a1, bb.lsb());
    bb.clear(.a1);
    try std.testing.expect(!bb.contains(.a1));
    try std.testing.expectEqual(@as(u7, 1), bb.count());
}

test "Bitboard iterator" {
    var bb = Bitboard.empty;
    bb.set(.a1);
    bb.set(.h8);
    bb.set(.e4);
    var iter = bb.iterator();
    var cnt: usize = 0;
    while (iter.next()) |sq| {
        _ = sq;
        cnt += 1;
    }
    try std.testing.expectEqual(@as(usize, 3), cnt);
}

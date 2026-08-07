const std = @import("std");

// ── File ─────────────────────────────────────────────────────────────────

pub const File = enum(u3) {
    a,
    b,
    c,
    d,
    e,
    f,
    g,
    h,

    pub const count = 8;

    pub inline fn fromChar(c: u8) ?File {
        if (c < 'a' or c > 'h') return null;
        return @enumFromInt(c - 'a');
    }

    pub inline fn char(self: File) u8 {
        return 'a' + @intFromEnum(self);
    }
};

// ── Rank ─────────────────────────────────────────────────────────────────

pub const Rank = enum(u3) {
    @"1",
    @"2",
    @"3",
    @"4",
    @"5",
    @"6",
    @"7",
    @"8",

    pub const count = 8;

    pub inline fn fromChar(c: u8) ?Rank {
        if (c < '1' or c > '8') return null;
        return @enumFromInt(c - '1');
    }

    pub inline fn char(self: Rank) u8 {
        return '1' + @intFromEnum(self);
    }
};

// ── Square ───────────────────────────────────────────────────────────────

pub const Square = enum(u6) {
    a1, b1, c1, d1, e1, f1, g1, h1,
    a2, b2, c2, d2, e2, f2, g2, h2,
    a3, b3, c3, d3, e3, f3, g3, h3,
    a4, b4, c4, d4, e4, f4, g4, h4,
    a5, b5, c5, d5, e5, f5, g5, h5,
    a6, b6, c6, d6, e6, f6, g6, h6,
    a7, b7, c7, d7, e7, f7, g7, h7,
    a8, b8, c8, d8, e8, f8, g8, h8,

    pub const count = 64;

    pub inline fn file(self: Square) File {
        return @enumFromInt(@intFromEnum(self) & 7);
    }

    pub inline fn rank(self: Square) Rank {
        return @enumFromInt(@intFromEnum(self) >> 3);
    }

    pub inline fn make(f: File, r: Rank) Square {
        return @enumFromInt((@as(u6, @intFromEnum(r)) << 3) | @as(u6, @intFromEnum(f)));
    }

    pub inline fn fromIndex(idx: u6) Square {
        return @enumFromInt(idx);
    }

    pub inline fn index(self: Square) u6 {
        return @intFromEnum(self);
    }

    pub inline fn isValid(idx: u8) bool {
        return idx < 64;
    }

    /// Flip vertically (rank 1 <-> 8).
    pub inline fn flipRank(self: Square) Square {
        return @enumFromInt(@intFromEnum(self) ^ 56);
    }

    /// Flip horizontally (file a <-> h).
    pub inline fn flipFile(self: Square) Square {
        return @enumFromInt(@intFromEnum(self) ^ 7);
    }

    pub fn name(self: Square) []const u8 {
        return names[@intFromEnum(self)];
    }

    pub fn fromName(n: []const u8) ?Square {
        if (n.len != 2) return null;
        if (n[0] < 'a' or n[0] > 'h') return null;
        if (n[1] < '1' or n[1] > '8') return null;
        return make(
            @enumFromInt(n[0] - 'a'),
            @enumFromInt(n[1] - '1'),
        );
    }

    const names = [_][]const u8{
        "a1", "b1", "c1", "d1", "e1", "f1", "g1", "h1",
        "a2", "b2", "c2", "d2", "e2", "f2", "g2", "h2",
        "a3", "b3", "c3", "d3", "e3", "f3", "g3", "h3",
        "a4", "b4", "c4", "d4", "e4", "f4", "g4", "h4",
        "a5", "b5", "c5", "d5", "e5", "f5", "g5", "h5",
        "a6", "b6", "c6", "d6", "e6", "f6", "g6", "h6",
        "a7", "b7", "c7", "d7", "e7", "f7", "g7", "h7",
        "a8", "b8", "c8", "d8", "e8", "f8", "g8", "h8",
    };
};

test "Square fromName and name round-trip" {
    try std.testing.expectEqualStrings("e4", Square.e4.name());
    try std.testing.expectEqual(Square.e4, Square.fromName("e4").?);
    try std.testing.expectEqual(@as(?Square, null), Square.fromName("i9"));
    try std.testing.expectEqual(@as(?Square, null), Square.fromName("e9"));
    try std.testing.expectEqual(@as(?Square, null), Square.fromName("a"));
    for (0..64) |i| {
        const sq: Square = @enumFromInt(i);
        const n = sq.name();
        try std.testing.expectEqual(sq, Square.fromName(n).?);
    }
}

test "Square file/rank/make" {
    try std.testing.expectEqual(File.e, Square.e4.file());
    try std.testing.expectEqual(Rank.@"4", Square.e4.rank());
    try std.testing.expectEqual(Square.e4, Square.make(.e, .@"4"));
    try std.testing.expectEqual(Square.a1.flipRank(), Square.a8);
    try std.testing.expectEqual(Square.a1.flipFile(), Square.h1);
}

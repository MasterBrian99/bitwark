const std = @import("std");

pub const CastlingRights = packed struct {
    white_kingside: bool = false,
    white_queenside: bool = false,
    black_kingside: bool = false,
    black_queenside: bool = false,

    pub const none: CastlingRights = .{};
    pub const all: CastlingRights = .{
        .white_kingside = true,
        .white_queenside = true,
        .black_kingside = true,
        .black_queenside = true,
    };

    pub inline fn isEmpty(self: CastlingRights) bool {
        return !self.white_kingside and !self.white_queenside and
            !self.black_kingside and !self.black_queenside;
    }

    pub fn toString(self: CastlingRights, buf: []u8) []const u8 {
        if (self.isEmpty()) {
            buf[0] = '-';
            return buf[0..1];
        }
        var len: usize = 0;
        if (self.white_kingside) {
            buf[len] = 'K';
            len += 1;
        }
        if (self.white_queenside) {
            buf[len] = 'Q';
            len += 1;
        }
        if (self.black_kingside) {
            buf[len] = 'k';
            len += 1;
        }
        if (self.black_queenside) {
            buf[len] = 'q';
            len += 1;
        }
        return buf[0..len];
    }

    pub fn fromString(s: []const u8) ?CastlingRights {
        if (s.len == 1 and s[0] == '-') return .none;
        var cr: CastlingRights = .none;
        for (s) |c| {
            switch (c) {
                'K' => cr.white_kingside = true,
                'Q' => cr.white_queenside = true,
                'k' => cr.black_kingside = true,
                'q' => cr.black_queenside = true,
                else => return null,
            }
        }
        return cr;
    }
};

test "CastlingRights toString/fromString" {
    var buf: [4]u8 = undefined;
    try std.testing.expectEqualStrings("-", CastlingRights.none.toString(&buf));
    try std.testing.expectEqualStrings("KQkq", CastlingRights.all.toString(&buf));
    try std.testing.expectEqual(CastlingRights.all, CastlingRights.fromString("KQkq").?);
    try std.testing.expectEqual(CastlingRights.none, CastlingRights.fromString("-").?);
    try std.testing.expectEqual(@as(?CastlingRights, null), CastlingRights.fromString("X"));
}

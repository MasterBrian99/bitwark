const std = @import("std");
const square_mod = @import("square.zig");
const piece_mod = @import("piece.zig");

pub const Square = square_mod.Square;
pub const PieceType = piece_mod.PieceType;

/// Compact move representation for learning. Explicit fields keep debugging easy;
/// later we can pack into u16 if needed. All flags are orthogonal.
pub const Move = struct {
    from: Square,
    to: Square,
    promotion: ?PieceType = null,
    is_capture: bool = false,
    is_en_passant: bool = false,
    is_castle: bool = false,
    is_double_push: bool = false,

    pub inline fn isPromotion(self: Move) bool {
        return self.promotion != null;
    }

    pub inline fn isQuiet(self: Move) bool {
        return !self.is_capture and !self.isPromotion();
    }

    /// UCI string, e.g. "e2e4", "e7e8q". Writes into buf (needs >=5).
    /// Returns slice of buf.
    pub fn toUci(self: Move, buf: []u8) []const u8 {
        const from_n = self.from.name();
        const to_n = self.to.name();
        buf[0] = from_n[0];
        buf[1] = from_n[1];
        buf[2] = to_n[0];
        buf[3] = to_n[1];
        if (self.promotion) |pt| {
            buf[4] = pt.char(); // 'q','r','b','n' — lower case per UCI
            return buf[0..5];
        }
        return buf[0..4];
    }

    /// Parse UCI move string "e2e4" or "e7e8q". Does not validate legality.
    /// Promotion char must be q/r/b/n (case-insensitive, stored lower).
    pub fn fromUci(s: []const u8) ?Move {
        if (s.len != 4 and s.len != 5) return null;
        const from = Square.fromName(s[0..2]) orelse return null;
        const to = Square.fromName(s[2..4]) orelse return null;
        var promo: ?PieceType = null;
        if (s.len == 5) {
            const c = s[4];
            const lower: u8 = if (c >= 'A' and c <= 'Z') c + 32 else c;
            promo = PieceType.fromChar(lower) orelse return null;
            // pawn promotions only to n/b/r/q
            switch (promo.?) {
                .knight, .bishop, .rook, .queen => {},
                else => return null,
            }
        }
        return .{ .from = from, .to = to, .promotion = promo };
    }

    pub fn eql(self: Move, other: Move) bool {
        return self.from == other.from and
            self.to == other.to and
            self.promotion == other.promotion and
            self.is_capture == other.is_capture and
            self.is_en_passant == other.is_en_passant and
            self.is_castle == other.is_castle and
            self.is_double_push == other.is_double_push;
    }
};

test "Move toUci / fromUci round-trip" {
    const m1 = Move{ .from = .e2, .to = .e4 };
    var buf: [5]u8 = undefined;
    try std.testing.expectEqualStrings("e2e4", m1.toUci(&buf));

    const m2 = Move{ .from = .e7, .to = .e8, .promotion = .queen };
    try std.testing.expectEqualStrings("e7e8q", m2.toUci(&buf));

    // parse
    try std.testing.expectEqual(Square.e2, Move.fromUci("e2e4").?.from);
    try std.testing.expectEqual(Square.e8, Move.fromUci("e7e8q").?.to);
    try std.testing.expectEqual(PieceType.queen, Move.fromUci("e7e8q").?.promotion.?);
    try std.testing.expectEqual(PieceType.knight, Move.fromUci("a7a8n").?.promotion.?);
    try std.testing.expectEqual(@as(?Move, null), Move.fromUci("e2e9"));
    try std.testing.expectEqual(@as(?Move, null), Move.fromUci("e2"));
    try std.testing.expectEqual(@as(?Move, null), Move.fromUci("e7e8k")); // king promo illegal
}

test "Move flags" {
    const m = Move{ .from = .e5, .to = .d6, .is_capture = true, .is_en_passant = true };
    try std.testing.expect(m.is_capture);
    try std.testing.expect(m.is_en_passant);
    try std.testing.expect(!m.isPromotion());
    const promo = Move{ .from = .a7, .to = .a8, .promotion = .rook };
    try std.testing.expect(promo.isPromotion());
}

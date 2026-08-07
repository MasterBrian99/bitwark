const std = @import("std");
const move_mod = @import("move.zig");
pub const Move = move_mod.Move;

/// Stack-only bounded list. Max legal moves is 218, 256 is safe margin.
/// No allocator in hot path (perft/search).
pub const MoveList = struct {
    moves: [256]Move = undefined,
    len: usize = 0,

    pub inline fn clear(self: *MoveList) void {
        self.len = 0;
    }

    pub inline fn push(self: *MoveList, m: Move) void {
        std.debug.assert(self.len < 256);
        self.moves[self.len] = m;
        self.len += 1;
    }

    pub inline fn at(self: MoveList, idx: usize) Move {
        std.debug.assert(idx < self.len);
        return self.moves[idx];
    }

    pub fn slice(self: *MoveList) []Move {
        return self.moves[0..self.len];
    }

    pub fn constSlice(self: *const MoveList) []const Move {
        return self.moves[0..self.len];
    }

    pub fn containsUci(self: MoveList, uci: []const u8) bool {
        var buf: [5]u8 = undefined;
        for (self.moves[0..self.len]) |m| {
            if (std.mem.eql(u8, m.toUci(&buf), uci)) return true;
        }
        return false;
    }
};

test "MoveList push and slice" {
    var list = MoveList{};
    try std.testing.expectEqual(@as(usize, 0), list.len);
    list.push(.{ .from = .e2, .to = .e4 });
    list.push(.{ .from = .g1, .to = .f3 });
    try std.testing.expectEqual(@as(usize, 2), list.len);
    try std.testing.expectEqual(Square.e4, list.at(0).to);
    list.clear();
    try std.testing.expectEqual(@as(usize, 0), list.len);
}

const Square = @import("square.zig").Square;

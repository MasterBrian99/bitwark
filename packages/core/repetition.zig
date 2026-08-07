const std = @import("std");
const board_mod = @import("board.zig");
pub const Board = board_mod.Board;

/// Simple repetition history tracking hashes. Detects threefold (and twofold).
/// Stores hashes of positions after each half-move. For now, checks full history linear scan.
pub const Repetition = struct {
    // Fixed capacity 1024 half-moves (~512 full moves) — enough for any game without 50-move.
    hashes: [1024]u64 = undefined,
    len: usize = 0,

    pub fn init() Repetition {
        return .{};
    }

    pub fn clear(self: *Repetition) void {
        self.len = 0;
    }

    pub fn push(self: *Repetition, board: Board) void {
        std.debug.assert(self.len < 1024);
        self.hashes[self.len] = board.hash;
        self.len += 1;
    }

    /// Push current board hash (convenience)
    pub fn pushHash(self: *Repetition, hash: u64) void {
        std.debug.assert(self.len < 1024);
        self.hashes[self.len] = hash;
        self.len += 1;
    }

    /// Remove last entry (for unmake)
    pub fn pop(self: *Repetition) void {
        std.debug.assert(self.len > 0);
        self.len -= 1;
    }

    /// Is current position a repetition? Checks if `hash` appears at least `needed` times in history.
    /// For threefold, `needed = 2` means we have seen it twice before and current makes third.
    pub fn isRepetition(self: Repetition, hash: u64) bool {
        var n: usize = 0;
        for (self.hashes[0..self.len]) |h| {
            if (h == hash) n += 1;
        }
        return n >= 2; // current position seen twice before → threefold including current (caller pushed current)
    }

    /// Count occurrences of hash in history (including current if pushed)
    pub fn count(self: Repetition, hash: u64) usize {
        var c: usize = 0;
        for (self.hashes[0..self.len]) |h| {
            if (h == hash) c += 1;
        }
        return c;
    }

    /// Threefold including current board (if board.hash already pushed)
    pub fn isThreefold(self: Repetition) bool {
        if (self.len == 0) return false;
        return self.count(self.hashes[self.len - 1]) >= 3;
    }
};

test "repetition push and detect" {
    var rep = Repetition.init();
    try std.testing.expect(!rep.isThreefold());
    rep.pushHash(0x1234);
    rep.pushHash(0x5678);
    rep.pushHash(0x1234);
    // not yet threefold
    try std.testing.expect(!rep.isThreefold());
    rep.pushHash(0x9abc);
    rep.pushHash(0x1234); // third time 0x1234 appears
    try std.testing.expect(rep.isThreefold());
    try std.testing.expectEqual(@as(usize, 3), rep.count(0x1234));
    rep.pop();
    try std.testing.expect(!rep.isThreefold());
}

test "repetition with board hashes" {
    const b = Board.startingPosition();
    var rep = Repetition.init();
    rep.push(b);
    const start_hash = b.hash;
    // Make a move and back (Nf3, Nf6, Ng1, Ng8) — should repeat startpos after 4 plies
    // We need simple moves; we will just push same hash manually for now
    rep.pushHash(start_hash);
    try std.testing.expect(!rep.isThreefold());
    rep.pushHash(start_hash);
    try std.testing.expect(rep.isThreefold());
}

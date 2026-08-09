const std = @import("std");
const core = @import("bitwark_core");

pub const Board = core.Board;

/// Transactional session: holds current board and allows begin/commit/rollback.
/// For now single level (no nesting), but API supports transactional updates.
pub const Session = struct {
    board: Board,
    // snapshot for rollback
    snapshot: ?Board = null,
    repetition: core.repetition.Repetition = .{},
    snapshot_rep: ?core.repetition.Repetition = null,

    pub fn init(board: Board) Session {
        var rep = core.repetition.Repetition.init();
        rep.push(board);
        return .{ .board = board, .repetition = rep };
    }

    pub fn begin(self: *Session) void {
        self.snapshot = self.board;
        self.snapshot_rep = self.repetition;
    }

    pub fn commit(self: *Session) void {
        self.snapshot = null;
        self.snapshot_rep = null;
    }

    pub fn rollback(self: *Session) void {
        if (self.snapshot) |snap| {
            self.board = snap;
            self.snapshot = null;
        }
        if (self.snapshot_rep) |snap| {
            self.repetition = snap;
            self.snapshot_rep = null;
        }
    }

    /// Apply FEN or startpos + moves transactionally. On error, rolls back and returns error.
    pub fn setPosition(self: *Session, fen_or_startpos: enum { startpos, fen }, fen_str: ?[]const u8, moves: []const []const u8) !void {
        self.begin();
        errdefer self.rollback();
        if (fen_or_startpos == .startpos) {
            self.board = Board.startingPosition();
        } else {
            const fen = fen_str orelse return error.InvalidFen;
            self.board = try core.fen.parseFen(fen);
        }
        // Rebuild repetition: clear → push initial → push after each applied move
        self.repetition.clear();
        self.repetition.push(self.board);
        for (moves) |uci| {
            const parsed = core.Move.fromUci(uci) orelse {
                return error.InvalidMove;
            };
            var list = core.MoveList{};
            core.movegen.generateLegal(self.board, &list);
            var found: ?core.Move = null;
            for (list.moves[0..list.len]) |m| {
                if (m.from == parsed.from and m.to == parsed.to and m.promotion == parsed.promotion) {
                    found = m;
                    break;
                }
                var buf: [5]u8 = undefined;
                if (std.mem.eql(u8, m.toUci(&buf), uci)) {
                    found = m;
                    break;
                }
            }
            if (found == null) return error.IllegalMove;
            core.movegen.applyMove(&self.board, found.?);
            self.repetition.push(self.board);
        }
        self.commit();
    }
};

test "session transactional" {
    var s = Session.init(Board.startingPosition());
    try s.setPosition(.startpos, null, &.{ "e2e4", "e7e5" });
    var fbuf: [128]u8 = undefined;
    const fen = core.fen.boardToFen(s.board, &fbuf);
    try std.testing.expect(std.mem.indexOf(u8, fen, "4P3") != null); // white pawn on e4, black pawn on e5
    // illegal move should rollback
    const before = s.board;
    const before_rep = s.repetition;
    const res = s.setPosition(.fen, "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", &.{ "e2e5" }); // illegal pawn push 3
    try std.testing.expectError(error.IllegalMove, res);
    try std.testing.expect(s.board.eql(before));
    // repetition should also rollback
    try std.testing.expectEqual(before_rep.len, s.repetition.len);
}

test "session repetition threefold" {
    var s = Session.init(Board.startingPosition());
    // Repetition via knight shuffling: g1f3 g8f6 f3g1 f6g8
    // After 8 plies we return to startpos 3 times? Let's use full cycle twice
    try s.setPosition(.startpos, null, &.{ "g1f3", "g8f6", "f3g1", "f6g8", "g1f3", "g8f6", "f3g1", "f6g8" });
    // startpos hash should appear 3 times in history (initial + after 4 plies + after 8 plies)
    try std.testing.expect(s.repetition.isThreefold());
}

test "session repetition reset on new position" {
    var s = Session.init(Board.startingPosition());
    try s.setPosition(.startpos, null, &.{ "g1f3", "g8f6", "f3g1", "f6g8" });
    // one repetition of startpos (count 2)
    try std.testing.expectEqual(@as(usize, 2), s.repetition.count(s.repetition.hashes[0]));
    // fresh position should reset to 1
    try s.setPosition(.fen, "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", &.{});
    try std.testing.expectEqual(@as(usize, 1), s.repetition.len);
    try std.testing.expect(!s.repetition.isThreefold());
}

test "session init repetition" {
    const s = Session.init(Board.startingPosition());
    try std.testing.expectEqual(@as(usize, 1), s.repetition.len);
    try std.testing.expectEqual(s.board.hash, s.repetition.hashes[0]);
}

const std = @import("std");
const core = @import("bitwark_core");

pub fn main(init: std.process.Init) !void {
    _ = init;
    const start_fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    const board = try core.fen.parseFen(start_fen);
    var buf: [128]u8 = undefined;
    const out = core.fen.boardToFen(board, &buf);
    std.debug.print("FEN ok: {s}\n", .{out});
}

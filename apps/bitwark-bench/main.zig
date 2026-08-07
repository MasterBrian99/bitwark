const std = @import("std");
const core = @import("bitwark_core");

pub fn main(init: std.process.Init) !void {
    _ = init;
    const start_fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    const board = try core.fen.parseFen(start_fen);
    _ = board;
    std.debug.print("bench: startpos parsed\n", .{});
}

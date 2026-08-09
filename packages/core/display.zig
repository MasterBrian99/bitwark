const std = @import("std");
const board_mod = @import("board.zig");
const fen_mod = @import("fen.zig");
const phase_mod = @import("phase.zig");

pub const Board = board_mod.Board;

/// Write human-readable board to writer (used by `d` and `bitwark-dump`).
pub fn writeBoard(writer: *std.Io.Writer, board: Board) !void {
    try writer.print("\n", .{});
    var r: i8 = 7;
    while (r >= 0) : (r -= 1) {
        try writer.print("{d} ", .{r + 1});
        var f: u4 = 0;
        while (f < 8) : (f += 1) {
            const sq = @import("square.zig").Square.make(@enumFromInt(@as(u3, @intCast(f))), @enumFromInt(@as(u3, @intCast(r))));
            if (board.pieceAt(sq)) |p| {
                try writer.print("{c} ", .{p.char()});
            } else {
                try writer.print(". ", .{});
            }
        }
        try writer.print("\n", .{});
    }
    try writer.print("  a b c d e f g h\n", .{});
    var cbuf: [4]u8 = undefined;
    try writer.print("Side: {s}  Castling: {s}  En passant: ", .{ board.side_to_move.name(), board.castling.toString(&cbuf) });
    if (board.en_passant) |ep| {
        try writer.print("{s}", .{ep.name()});
    } else {
        try writer.print("-", .{});
    }
    try writer.print("  Halfmove: {d}  Fullmove: {d}  Hash: 0x{x:0>16}  Phase: {s}\n", .{ board.halfmove_clock, board.fullmove_number, board.hash, @tagName(phase_mod.classify(board)) });
    var fbuf: [128]u8 = undefined;
    try writer.print("FEN: {s}\n", .{fen_mod.boardToFen(board, &fbuf)});
}

test "display writeBoard" {
    var buf: [1024]u8 = undefined;
    var w: std.Io.Writer = .fixed(&buf);
    try writeBoard(&w, Board.startingPosition());
    try std.testing.expect(w.end > 0);
}

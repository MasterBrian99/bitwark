const std = @import("std");

// Simple event publication: engine publishes `info` and `bestmove` lines to stdout.
// For now just helpers to format UCI output. Later this can be an async channel.

pub fn publishInfo(io: std.Io, writer: *std.Io.File.Writer, depth: u8, score: i16, nodes: u64, pv_uci: []const u8) !void {
    try writer.interface.print("info depth {d} score cp {d} nodes {d} pv {s}\n", .{ depth, score, nodes, pv_uci });
    try writer.interface.flush();
    _ = io;
}

pub fn publishBestmove(io: std.Io, writer: *std.Io.File.Writer, uci: []const u8) !void {
    try writer.interface.print("bestmove {s}\n", .{uci});
    try writer.interface.flush();
    _ = io;
}

pub fn publishId(io: std.Io, writer: *std.Io.File.Writer) !void {
    try writer.interface.print("id name bitwark 0.1.0\n", .{});
    try writer.interface.print("id author bitwark\n", .{});
    try writer.interface.flush();
    _ = io;
}

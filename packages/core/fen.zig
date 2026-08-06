const std = @import("std");
const Io = std.Io;

pub fn fen_hello() !void {
    std.debug.print(" {s}.\n", .{"Cring"});
}

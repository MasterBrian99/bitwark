const std = @import("std");
const builtin = @import("builtin");

pub fn writeCompilerInfo(writer: *std.Io.Writer) !void {
    const ver = builtin.zig_version;
    try writer.print("info string compiler zig {d}.{d}.{d} {s}-{s} {s} cpu={s}\n", .{
        ver.major,
        ver.minor,
        ver.patch,
        @tagName(builtin.target.cpu.arch),
        @tagName(builtin.target.os.tag),
        @tagName(builtin.mode),
        @tagName(builtin.cpu.arch),
    });
}

test "compiler info" {
    var buf: [256]u8 = undefined;
    var w: std.Io.Writer = .fixed(&buf);
    try writeCompilerInfo(&w);
    try std.testing.expect(w.end > 0);
}

const std = @import("std");
const core = @import("bitwark_core");

const VERSION = "0.1.0";
const usage =
    \\bitwark-eval — classical evaluation breakdown tool
    \\
    \\Usage: bitwark-eval [options]
    \\       bitwark-eval --fen <FEN> [--breakdown] [--stm]
    \\
    \\Options:
    \\  --fen <FEN>     FEN position (default: startpos)
    \\  --breakdown     Show 13-term breakdown
    \\  --stm           Show score from side-to-move perspective (negamax)
    \\  --help          Show this help to stdout
    \\  --version       Show version to stdout
    \\
    \\Exit codes:
    \\  0 success, 1 usage error, 2 FEN parse error
    \\
;

pub fn main(init: std.process.Init) !void {
    const arena = init.arena.allocator();
    const args = try init.minimal.args.toSlice(arena);
    const io = init.io;
    var stdout_buf: [4096]u8 = undefined;
    var stdout_w: std.Io.File.Writer = .init(.stdout(), io, &stdout_buf);
    var stderr_buf: [4096]u8 = undefined;
    var stderr_w: std.Io.File.Writer = .init(.stderr(), io, &stderr_buf);

    var fen_opt: ?[]const u8 = null;
    var breakdown = false;
    var stm = false;
    var i: usize = 1;
    while (i < args.len) : (i += 1) {
        const a = args[i];
        if (std.mem.eql(u8, a, "--help") or std.mem.eql(u8, a, "-h")) {
            try stdout_w.interface.writeAll(usage);
            try stdout_w.interface.flush();
            std.process.exit(0);
        } else if (std.mem.eql(u8, a, "--version") or std.mem.eql(u8, a, "-V")) {
            try stdout_w.interface.print("{s}\n", .{VERSION});
            try stdout_w.interface.flush();
            std.process.exit(0);
        } else if (std.mem.eql(u8, a, "--fen")) {
            i += 1;
            if (i >= args.len) {
                try stderr_w.interface.print("error: --fen requires argument\n", .{});
                try stderr_w.interface.flush();
                std.process.exit(1);
            }
            fen_opt = args[i];
        } else if (std.mem.eql(u8, a, "--breakdown")) {
            breakdown = true;
        } else if (std.mem.eql(u8, a, "--stm")) {
            stm = true;
        } else {
            try stderr_w.interface.print("error: unknown argument '{s}'\n", .{a});
            try stderr_w.interface.print("try --help\n", .{});
            try stderr_w.interface.flush();
            std.process.exit(1);
        }
    }

    const fen_str = fen_opt orelse "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    const board = core.fen.parseFen(fen_str) catch |err| {
        try stderr_w.interface.print("error: invalid FEN: {t}\n", .{err});
        try stderr_w.interface.flush();
        std.process.exit(2);
    };

    const bd = core.eval.evaluate(board);
    const total = bd.total();
    const stm_score = core.eval.evaluateForSide(board);

    if (breakdown) {
        for (core.eval.term_names, bd.toArray()) |name, val| {
            try stdout_w.interface.print("{s:20}: {d:5}\n", .{ name, val });
        }
        try stdout_w.interface.print("{s:20}: {d:5}\n", .{ "total (white)", total });
        try stdout_w.interface.print("{s:20}: {d:5}\n", .{ "total (stm)", stm_score });
        try stdout_w.interface.flush();
    } else {
        const out: i32 = if (stm) stm_score else total;
        try stdout_w.interface.print("{d}\n", .{out});
        try stdout_w.interface.flush();
    }
}

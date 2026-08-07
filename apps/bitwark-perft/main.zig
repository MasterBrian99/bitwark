const std = @import("std");
const core = @import("bitwark_core");

const VERSION = "0.1.0";

const usage =
    \\bitwark-perft — move-generation correctness and perft tool
    \\
    \\Usage: bitwark-perft [options]
    \\       bitwark-perft --fen <FEN> --depth <N> [--divide]
    \\
    \\Options:
    \\  --fen <FEN>     FEN position (default: startpos)
    \\  --depth <N>     Depth to count (required, 0..7)
    \\  --divide        Show per-move breakdown (divide)
    \\  --help          Show this help to stdout
    \\  --version       Show version to stdout
    \\
    \\Exit codes:
    \\  0 success, 1 usage error, 2 FEN parse error
    \\
;

fn printHelp(io: std.Io, writer: *std.Io.File.Writer) !void {
    try writer.interface.writeAll(usage);
    try writer.interface.flush();
    _ = io;
}

fn printVersion(io: std.Io, writer: *std.Io.File.Writer) !void {
    try writer.interface.print("{s}\n", .{VERSION});
    try writer.interface.flush();
    _ = io;
}

pub fn main(init: std.process.Init) !void {
    const arena = init.arena.allocator();
    const args = try init.minimal.args.toSlice(arena);
    const io = init.io;

    var stdout_buf: [4096]u8 = undefined;
    var stdout_w: std.Io.File.Writer = .init(.stdout(), io, &stdout_buf);
    var stderr_buf: [4096]u8 = undefined;
    var stderr_w: std.Io.File.Writer = .init(.stderr(), io, &stderr_buf);

    var fen_opt: ?[]const u8 = null;
    var depth_opt: ?u32 = null;
    var divide = false;
    var i: usize = 1;
    while (i < args.len) : (i += 1) {
        const a = args[i];
        if (std.mem.eql(u8, a, "--help") or std.mem.eql(u8, a, "-h")) {
            try printHelp(io, &stdout_w);
            std.process.exit(0);
        } else if (std.mem.eql(u8, a, "--version") or std.mem.eql(u8, a, "-V")) {
            try printVersion(io, &stdout_w);
            std.process.exit(0);
        } else if (std.mem.eql(u8, a, "--fen")) {
            i += 1;
            if (i >= args.len) {
                try stderr_w.interface.print("error: --fen requires argument\n", .{});
                try stderr_w.interface.flush();
                std.process.exit(1);
            }
            fen_opt = args[i];
        } else if (std.mem.eql(u8, a, "--depth")) {
            i += 1;
            if (i >= args.len) {
                try stderr_w.interface.print("error: --depth requires argument\n", .{});
                try stderr_w.interface.flush();
                std.process.exit(1);
            }
            depth_opt = std.fmt.parseInt(u32, args[i], 10) catch {
                try stderr_w.interface.print("error: invalid depth '{s}'\n", .{args[i]});
                try stderr_w.interface.flush();
                std.process.exit(1);
            };
        } else if (std.mem.eql(u8, a, "--divide")) {
            divide = true;
        } else {
            try stderr_w.interface.print("error: unknown argument '{s}'\n", .{a});
            try stderr_w.interface.print("try --help\n", .{});
            try stderr_w.interface.flush();
            std.process.exit(1);
        }
    }

    if (depth_opt == null) {
        try stderr_w.interface.print("error: --depth is required\n", .{});
        try stderr_w.interface.print("try --help\n", .{});
        try stderr_w.interface.flush();
        std.process.exit(1);
    }
    const depth = depth_opt.?;

    const fen_str = fen_opt orelse "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    const board = core.fen.parseFen(fen_str) catch |err| {
        try stderr_w.interface.print("error: invalid FEN: {t}\n", .{err});
        try stderr_w.interface.flush();
        std.process.exit(2);
    };

    if (divide) {
        var buf: [256]core.perft.DivideEntry = undefined;
        const n = core.perft.perftDivide(board, depth, &buf);
        var total: u64 = 0;
        for (buf[0..n]) |e| {
            const uci = e.uci[0..e.len];
            try stdout_w.interface.print("{s}: {d}\n", .{ uci, e.nodes });
            total += e.nodes;
        }
        try stdout_w.interface.print("\n{d}\n", .{total});
        try stdout_w.interface.flush();
    } else {
        const nodes = core.perft.perft(board, depth);
        try stdout_w.interface.print("{d}\n", .{nodes});
        try stdout_w.interface.flush();
    }
}

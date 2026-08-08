const std = @import("std");
const core = @import("bitwark_core");

const VERSION = "0.1.0";
const usage =
    \\bitwark-replay — move-sequence replay and state-verification tool
    \\
    \\Usage: bitwark-replay [options] --moves <uci1> [uci2 ...]
    \\       bitwark-replay --fen <FEN> --moves e2e4 e7e5 g1f3
    \\
    \\Options:
    \\  --fen <FEN>     Start FEN (default: startpos)
    \\  --moves <list>  UCI moves to replay (required)
    \\  --help          Show this help to stdout
    \\  --version       Show version to stdout
    \\
    \\Exit codes:
    \\  0 success, 1 usage error, 2 FEN/ move error
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
    var moves_start: ?usize = null;
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
        } else if (std.mem.eql(u8, a, "--moves")) {
            moves_start = i + 1;
            break;
        } else {
            try stderr_w.interface.print("error: unknown argument '{s}'\n", .{a});
            try stderr_w.interface.print("try --help\n", .{});
            try stderr_w.interface.flush();
            std.process.exit(1);
        }
    }

    if (moves_start == null) {
        try stderr_w.interface.print("error: --moves is required\n", .{});
        try stderr_w.interface.print("try --help\n", .{});
        try stderr_w.interface.flush();
        std.process.exit(1);
    }

    const fen_str = fen_opt orelse "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    var board = core.fen.parseFen(fen_str) catch |err| {
        try stderr_w.interface.print("error: invalid FEN: {t}\n", .{err});
        try stderr_w.interface.flush();
        std.process.exit(2);
    };

    const move_strs = args[moves_start.?..];
    for (move_strs) |uci| {
        const parsed = core.Move.fromUci(uci) orelse {
            try stderr_w.interface.print("error: invalid UCI '{s}'\n", .{uci});
            try stderr_w.interface.flush();
            std.process.exit(2);
        };
        // Find matching legal move (to get flags)
        var list = core.MoveList{};
        core.movegen.generateLegal(board, &list);
        var found: ?core.Move = null;
        var buf: [5]u8 = undefined;
        for (list.moves[0..list.len]) |m| {
            if (std.mem.eql(u8, m.toUci(&buf), uci)) {
                // Also allow promotion char case-insensitive: fromUci already handles, but we matched via toUci lower
                found = m;
                break;
            }
            // Also try matching just from/to ignoring flags via parsed
            if (m.from == parsed.from and m.to == parsed.to and m.promotion == parsed.promotion) {
                found = m;
                break;
            }
        }
        if (found == null) {
            try stderr_w.interface.print("error: illegal move '{s}' in position\n", .{uci});
            try stderr_w.interface.flush();
            std.process.exit(2);
        }
        core.movegen.applyMove(&board, found.?);
    }

    var fbuf: [128]u8 = undefined;
    const out = core.fen.boardToFen(board, &fbuf);
    try stdout_w.interface.print("{s}\n", .{out});
    try stdout_w.interface.flush();
}

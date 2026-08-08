const std = @import("std");
const core = @import("bitwark_core");

const VERSION = "0.1.0";
const usage =
    \\bitwark-dump — board and internal-state inspection tool
    \\
    \\Usage: bitwark-dump [options]
    \\       bitwark-dump --fen <FEN> [--fen-out]
    \\
    \\Options:
    \\  --fen <FEN>     FEN position (default: startpos)
    \\  --fen-out       Also print FEN round-trip to stdout after dump
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
    var fen_out = false;
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
        } else if (std.mem.eql(u8, a, "--fen-out")) {
            fen_out = true;
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

    // Board ASCII to stdout (reserved output)
    try stdout_w.interface.print("\n", .{});
    var r: i8 = 7;
    while (r >= 0) : (r -= 1) {
        try stdout_w.interface.print("{d} ", .{r + 1});
        var f: u4 = 0;
        while (f < 8) : (f += 1) {
            const sq = core.Square.make(@enumFromInt(@as(u3, @intCast(f))), @enumFromInt(@as(u3, @intCast(r))));
            if (board.pieceAt(sq)) |p| {
                try stdout_w.interface.print("{c} ", .{p.char()});
            } else {
                try stdout_w.interface.print(". ", .{});
            }
        }
        try stdout_w.interface.print("\n", .{});
    }
    try stdout_w.interface.print("  a b c d e f g h\n", .{});
    var cbuf: [4]u8 = undefined;
    const cs = board.castling.toString(&cbuf);
    try stdout_w.interface.print("Side: {s}  Castling: {s}  En passant: ", .{ board.side_to_move.name(), cs });
    if (board.en_passant) |ep| {
        try stdout_w.interface.print("{s}", .{ep.name()});
    } else {
        try stdout_w.interface.print("-", .{});
    }
    try stdout_w.interface.print("  Halfmove: {d}  Fullmove: {d}  Hash: 0x{x:0>16}\n", .{ board.halfmove_clock, board.fullmove_number, board.hash });

    if (fen_out) {
        var fbuf: [128]u8 = undefined;
        const out = core.fen.boardToFen(board, &fbuf);
        try stdout_w.interface.print("FEN: {s}\n", .{out});
    }
    try stdout_w.interface.flush();
}

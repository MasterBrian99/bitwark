const std = @import("std");
const core = @import("bitwark_core");
const proto = @import("bitwark_protocol");

const VERSION = "0.1.0";
const usage =
    \\bitwark-cli — human-friendly interactive shell (thin)
    \\Commands:
    \\  pos startpos [moves ...] | pos fen <FEN> [moves ...]
    \\  go depth <N> | go movetime <MS>
    \\  eval [--fen <FEN>]
    \\  perft <depth> [--fen <FEN>]
    \\  dump [--fen <FEN>]
    \\  help | quit
    \\
;

pub fn main(init: std.process.Init) !void {
    const arena = init.arena.allocator();
    const io = init.io;
    var stdout_buf: [8192]u8 = undefined;
    var stdout_w: std.Io.File.Writer = .init(.stdout(), io, &stdout_buf);
    var stderr_buf: [1024]u8 = undefined;
    var stderr_w: std.Io.File.Writer = .init(.stderr(), io, &stderr_buf);

    var session = proto.session.Session.init(core.Board.startingPosition());

    try stdout_w.interface.print("bitwark-cli {s} — type help\n", .{VERSION});
    try stdout_w.interface.flush();

    var stdin_buf: [8192]u8 = undefined;
    var stdin_reader: std.Io.File.Reader = .init(.stdin(), io, &stdin_buf);
    const stdin = &stdin_reader.interface;
    while (true) {
        try stdout_w.interface.print("> ", .{});
        try stdout_w.interface.flush();
        const maybe_line = stdin.takeDelimiter('\n') catch |err| {
            if (err == error.StreamTooLong) {
                try stderr_w.interface.print("line too long\n", .{});
                try stderr_w.interface.flush();
                _ = stdin.takeDelimiterInclusive('\n') catch {};
                continue;
            }
            try stderr_w.interface.print("read failed: {t}\n", .{err});
            try stderr_w.interface.flush();
            break;
        };
        const line = maybe_line orelse break;
        const trimmed = std.mem.trim(u8, line, &std.ascii.whitespace);
        if (trimmed.len == 0) continue;
        if (std.mem.eql(u8, trimmed, "quit") or std.mem.eql(u8, trimmed, "exit")) break;
        if (std.mem.eql(u8, trimmed, "help")) {
            try stdout_w.interface.writeAll(usage);
            try stdout_w.interface.flush();
            continue;
        }
        if (std.mem.startsWith(u8, trimmed, "pos ")) {
            const rest = trimmed["pos ".len..];
            var arena2 = std.heap.ArenaAllocator.init(arena);
            defer arena2.deinit();
            var tmp: [8192]u8 = undefined;
            const full = std.fmt.bufPrint(&tmp, "position {s}", .{rest}) catch {
                try stderr_w.interface.print("line too long\n", .{});
                try stderr_w.interface.flush();
                continue;
            };
            const cmd = proto.uci.parseLine(full, arena2.allocator()) catch {
                try stderr_w.interface.print("parse error\n", .{});
                try stderr_w.interface.flush();
                continue;
            };
            switch (cmd) {
                .position_startpos => |p| {
                    session.setPosition(.startpos, null, p.moves) catch |e| {
                        try stderr_w.interface.print("error: {t}\n", .{e});
                        try stderr_w.interface.flush();
                    };
                },
                .position_fen => |p| {
                    session.setPosition(.fen, p.fen, p.moves) catch |e| {
                        try stderr_w.interface.print("error: {t}\n", .{e});
                        try stderr_w.interface.flush();
                    };
                },
                else => {
                    try stderr_w.interface.print("unknown pos\n", .{});
                    try stderr_w.interface.flush();
                },
            }
            var fbuf: [128]u8 = undefined;
            try stdout_w.interface.print("pos: {s}\n", .{core.fen.boardToFen(session.board, &fbuf)});
            try stdout_w.interface.flush();
            continue;
        }
        if (std.mem.startsWith(u8, trimmed, "go ")) {
            const rest = trimmed["go ".len..];
            var depth: u8 = 3;
            if (std.mem.startsWith(u8, rest, "depth ")) {
                depth = std.fmt.parseInt(u8, rest["depth ".len..], 10) catch 3;
            }
            const res = core.search.search(session.board, .{ .depth = depth });
            if (res.bestmove) |bm| {
                var buf: [5]u8 = undefined;
                try stdout_w.interface.print("bestmove {s} score {d} nodes {d} {s}\n", .{ bm.toUci(&buf), res.score, res.nodes, if (res.from_book) "(book)" else "" });
                try stdout_w.interface.flush();
                core.movegen.applyMove(&session.board, bm);
            } else {
                try stdout_w.interface.print("no legal moves\n", .{});
                try stdout_w.interface.flush();
            }
            continue;
        }
        if (std.mem.startsWith(u8, trimmed, "eval")) {
            const bd = core.eval.evaluate(session.board);
            try stdout_w.interface.print("eval {d} (white) / {d} (stm)\n", .{ bd.total(), core.eval.evaluateForSide(session.board) });
            for (core.eval.term_names, bd.toArray()) |n, v| {
                try stdout_w.interface.print("  {s:20} {d:5}\n", .{ n, v });
            }
            try stdout_w.interface.flush();
            continue;
        }
        if (std.mem.startsWith(u8, trimmed, "perft")) {
            const rest = std.mem.trim(u8, trimmed["perft".len..], &std.ascii.whitespace);
            const d = std.fmt.parseInt(u32, rest, 10) catch {
                try stderr_w.interface.print("usage: perft <depth>\n", .{});
                try stderr_w.interface.flush();
                continue;
            };
            const nodes = core.perft.perft(session.board, d);
            try stdout_w.interface.print("{d}\n", .{nodes});
            try stdout_w.interface.flush();
            continue;
        }
        if (std.mem.startsWith(u8, trimmed, "dump")) {
            var fbuf: [128]u8 = undefined;
            try stdout_w.interface.print("FEN: {s}\n", .{core.fen.boardToFen(session.board, &fbuf)});
            try stdout_w.interface.print("hash: 0x{x:0>16} phase: {t}\n", .{ session.board.hash, core.phase.classify(session.board) });
            try stdout_w.interface.flush();
            continue;
        }
        try stderr_w.interface.print("unknown: {s} (try help)\n", .{trimmed});
        try stderr_w.interface.flush();
    }
}

const std = @import("std");
const core = @import("bitwark_core");
const proto = @import("bitwark_protocol");

const VERSION = "0.1.0";

const usage =
    \\bitwarkd — UCI daemon (phase-aware, thin frontend for bitwark engine)
    \\
    \\Usage: bitwarkd [options]
    \\       echo "uci\nisready\nposition startpos\ngo depth 4\nquit" | bitwarkd
    \\
    \\Options:
    \\  --help          Show this help to stdout
    \\  --version       Show version to stdout
    \\
    \\UCI protocol (stdin/stdout, diagnostics to stderr):
    \\  uci, isready, ucinewgame, position [startpos|fen ...] [moves ...], go depth <N>, go movetime <ms>, go infinite, stop, quit
    \\  Responses: id name, uciok, readyok, bestmove, info string phase <opening|middlegame|endgame>
    \\
    \\Exit codes:
    \\  0 success, 1 usage error
    \\
;

fn printHelp(io: std.Io, writer: *std.Io.File.Writer) !void {
    try writer.interface.writeAll(usage);
    try writer.interface.flush();
    _ = io;
}

fn printVersion(io: std.Io, writer: *std.Io.File.Writer) !void {
    try writer.interface.print("bitwarkd {s}\n", .{VERSION});
    try writer.interface.flush();
    _ = io;
}

pub fn main(init: std.process.Init) !void {
    const arena = init.arena.allocator();
    const args = try init.minimal.args.toSlice(arena);
    const io = init.io;

    var stdout_buf: [8192]u8 = undefined;
    var stdout_w: std.Io.File.Writer = .init(.stdout(), io, &stdout_buf);
    var stderr_buf: [1024]u8 = undefined;
    var stderr_w: std.Io.File.Writer = .init(.stderr(), io, &stderr_buf);

    // --help/--version before UCI loop
    if (args.len > 1) {
        for (args[1..]) |a| {
            if (std.mem.eql(u8, a, "--help") or std.mem.eql(u8, a, "-h")) {
                try printHelp(io, &stdout_w);
                return;
            } else if (std.mem.eql(u8, a, "--version") or std.mem.eql(u8, a, "-V")) {
                try printVersion(io, &stdout_w);
                return;
            } else {
                try stderr_w.interface.print("unknown option: {s}\n{s}", .{ a, usage });
                try stderr_w.interface.flush();
                std.process.exit(1);
            }
        }
    }

    var session = proto.session.Session.init(core.Board.startingPosition());
    var controller = proto.controller.Controller{};
    var tt = proto.table.allocTT(arena, 1024 * 1024) catch {
        try stderr_w.interface.print("error: TT alloc failed\n", .{});
        try stderr_w.interface.flush();
        return;
    };
    defer tt.deinit(arena);

    var stdin_buf: [8192]u8 = undefined;
    var stdin_reader: std.Io.File.Reader = .init(.stdin(), io, &stdin_buf);
    const stdin = &stdin_reader.interface;

    while (true) {
        const maybe_line = stdin.takeDelimiter('\n') catch |err| {
            if (err == error.StreamTooLong) {
                try stderr_w.interface.print("error: line too long\n", .{});
                try stderr_w.interface.flush();
                _ = stdin.takeDelimiterInclusive('\n') catch {};
                continue;
            }
            try stderr_w.interface.print("error: read failed: {t}\n", .{err});
            try stderr_w.interface.flush();
            break;
        };
        const line = maybe_line orelse break;
        const trimmed = std.mem.trim(u8, line, &std.ascii.whitespace);
        if (trimmed.len == 0) continue;

        var line_arena = std.heap.ArenaAllocator.init(arena);
        defer line_arena.deinit();

        const cmd = proto.uci.parseLine(trimmed, line_arena.allocator()) catch {
            try stderr_w.interface.print("error: line too long or empty\n", .{});
            try stderr_w.interface.flush();
            continue;
        };

        switch (cmd) {
            .uci => {
                try proto.events.publishId(io, &stdout_w);
                try stdout_w.interface.print("uciok\n", .{});
                try stdout_w.interface.flush();
            },
            .isready => {
                try stdout_w.interface.print("readyok\n", .{});
                try stdout_w.interface.flush();
            },
            .ucinewgame => {
                session = proto.session.Session.init(core.Board.startingPosition());
                tt.clear();
            },
            .quit => break,
            .stop => {
                if (controller.isBusy()) controller.endSearch();
            },
            .position_startpos => |p| {
                const ok = session.setPosition(.startpos, null, p.moves) catch |err| {
                    try stderr_w.interface.print("error: position: {t}\n", .{err});
                    try stderr_w.interface.flush();
                    continue;
                };
                _ = ok;
            },
            .position_fen => |p| {
                const ok = session.setPosition(.fen, p.fen, p.moves) catch |err| {
                    try stderr_w.interface.print("error: position: {t}\n", .{err});
                    try stderr_w.interface.flush();
                    continue;
                };
                _ = ok;
            },
            .go_depth => |d| {
                if (!controller.startSearch()) {
                    try stderr_w.interface.print("info string busy\n", .{});
                    try stderr_w.interface.flush();
                    continue;
                }
                defer controller.endSearch();
                const limits = core.search.SearchLimits{ .depth = d };
                const res = core.search.searchWithCancellation(session.board, limits, .{ .cancelled = &struct { var dummy: bool = false; }.dummy });
                if (res.bestmove) |bm| {
                    var buf: [5]u8 = undefined;
                    const uci = bm.toUci(&buf);
                    if (res.from_book) {
                        try stdout_w.interface.print("info string book hit {s}\n", .{uci});
                    } else {
                        const ph = core.phase.classify(session.board);
                        try stdout_w.interface.print("info string phase {s}\n", .{@tagName(ph)});
                    }
                    try proto.events.publishBestmove(io, &stdout_w, uci);
                } else {
                    try stdout_w.interface.print("bestmove 0000\n", .{});
                    try stdout_w.interface.flush();
                }
            },
            .go_movetime => |ms| {
                if (!controller.startSearch()) {
                    try stderr_w.interface.print("info string busy\n", .{});
                    try stderr_w.interface.flush();
                    continue;
                }
                defer controller.endSearch();
                _ = ms;
                const limits = core.search.SearchLimits{ .depth = 4 };
                const res = core.search.searchWithCancellation(session.board, limits, .{ .cancelled = &struct { var dummy: bool = false; }.dummy });
                if (res.bestmove) |bm| {
                    var buf: [5]u8 = undefined;
                    try proto.events.publishBestmove(io, &stdout_w, bm.toUci(&buf));
                } else {
                    try stdout_w.interface.print("bestmove 0000\n", .{});
                    try stdout_w.interface.flush();
                }
            },
            .go_infinite => {
                if (!controller.startSearch()) {
                    try stderr_w.interface.print("info string busy\n", .{});
                    try stderr_w.interface.flush();
                    continue;
                }
                defer controller.endSearch();
                const limits = core.search.SearchLimits{ .depth = 4 };
                const res = core.search.searchWithCancellation(session.board, limits, .{ .cancelled = &struct { var dummy: bool = false; }.dummy });
                if (res.bestmove) |bm| {
                    var buf: [5]u8 = undefined;
                    try proto.events.publishBestmove(io, &stdout_w, bm.toUci(&buf));
                } else {
                    try stdout_w.interface.print("bestmove 0000\n", .{});
                    try stdout_w.interface.flush();
                }
            },
            .unknown => |s| {
                try stderr_w.interface.print("unknown command: {s}\n", .{s});
                try stderr_w.interface.flush();
            },
        }
    }
}

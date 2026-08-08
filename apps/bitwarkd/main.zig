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
    \\UCI protocol (full spec, stdin/stdout, diagnostics to stderr):
    \\  uci, debug [on|off], isready, setoption name <id> [value <x>], register [later|name|code],
    \\  ucinewgame, position [startpos|fen <fen>] [moves ...],
    \\  go [searchmoves ...] [ponder] [wtime|btime|winc|binc|movestogo] [depth|nodes|mate|movetime|infinite],
    \\  stop, ponderhit, quit
    \\  Responses: id name/author, option ..., uciok, readyok, bestmove [ponder], copyprotection, registration, info ...
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
    const io = init.io;

    var stdout_buf: [8192]u8 = undefined;
    var stdout_w: std.Io.File.Writer = .init(.stdout(), io, &stdout_buf);
    var stderr_buf: [1024]u8 = undefined;
    var stderr_w: std.Io.File.Writer = .init(.stderr(), io, &stderr_buf);

    // --help/--version before UCI loop (daemon contract)
    {
        const args = try init.minimal.args.toSlice(arena);
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
    }

    var session = proto.session.Session.init(core.Board.startingPosition());
    var controller = proto.controller.Controller{};
    var tt = proto.table.allocTT(arena, 1024 * 1024) catch {
        try stderr_w.interface.print("error: TT alloc failed\n", .{});
        try stderr_w.interface.flush();
        return;
    };
    defer tt.deinit(arena);

    var debug_on = false;
    var threads: u8 = 1;
    var hash_mb: u32 = 16;

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
            .debug => |on| {
                debug_on = on;
                try proto.events.publishInfoString(io, &stdout_w, if (on) "debug on" else "debug off");
            },
            .isready => {
                try proto.events.publishReadyOk(io, &stdout_w);
            },
            .setoption => |opt| {
                if (std.mem.eql(u8, opt.name, "Threads")) {
                    if (opt.value) |v| {
                        threads = std.fmt.parseInt(u8, v, 10) catch threads;
                        if (threads < 1) threads = 1;
                        if (threads > 16) threads = 16;
                    }
                    try proto.events.publishInfoString(io, &stdout_w, "Threads set");
                } else if (std.mem.eql(u8, opt.name, "Hash")) {
                    if (opt.value) |v| {
                        hash_mb = std.fmt.parseInt(u32, v, 10) catch hash_mb;
                    }
                    try proto.events.publishInfoString(io, &stdout_w, "Hash set");
                } else if (std.mem.eql(u8, opt.name, "Ponder") or std.mem.eql(u8, opt.name, "UCI_Chess960") or std.mem.eql(u8, opt.name, "MultiPV")) {
                    try proto.events.publishInfoString(io, &stdout_w, "option ok");
                } else {
                    try stderr_w.interface.print("unknown option: {s}\n", .{opt.name});
                    try stderr_w.interface.flush();
                }
            },
            .register => |reg| {
                if (reg.later) {
                    try proto.events.publishRegistration(io, &stdout_w, "later");
                } else if (reg.name != null or reg.code != null) {
                    try proto.events.publishRegistration(io, &stdout_w, "checking");
                    try proto.events.publishRegistration(io, &stdout_w, "ok");
                } else {
                    try proto.events.publishRegistration(io, &stdout_w, "ok");
                }
            },
            .ucinewgame => {
                session = proto.session.Session.init(core.Board.startingPosition());
                tt.clear();
            },
            .quit => break,
            .stop => {
                if (controller.isBusy()) controller.endSearch();
            },
            .ponderhit => {
                try proto.events.publishInfoString(io, &stdout_w, "ponderhit received");
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
            .go => |g| {
                if (!controller.startSearch()) {
                    try stderr_w.interface.print("info string busy\n", .{});
                    try stderr_w.interface.flush();
                    continue;
                }
                defer controller.endSearch();

                // Map UCI go params to search limits (small stubs for now)
                var depth: u8 = 3;
                const nodes = g.nodes;
                const threads_use = threads;

                if (g.depth) |d| depth = d;
                if (g.movetime) |ms| {
                    // Simple: movetime 0..50 -> depth 2, 50..200 ->3, 200..1000 ->4, else 5
                    depth = if (ms < 50) 2 else if (ms < 200) 3 else if (ms < 1000) 4 else 5;
                }
                if (g.wtime != null or g.btime != null) {
                    // If time control, use depth 3 or 4 based on remaining time
                    if (depth == 3 and g.depth == null and g.movetime == null and g.mate == null and g.infinite == false and g.nodes == null) {
                        const stm = session.board.side_to_move;
                        const my_time = if (stm == .white) g.wtime else g.btime;
                        const my_inc = if (stm == .white) g.winc else g.binc;
                        if (my_time) |t| {
                            if (t < 1000) depth = 2
                            else if (t < 5000) depth = 3
                            else depth = 4;
                            _ = my_inc;
                        }
                    }
                    _ = g.movestogo;
                }
                if (g.mate) |m| {
                    // Mate search: depth = mate * 2 (small stub)
                    depth = @min(m * 2, 6);
                }
                if (g.infinite and g.depth == null and g.movetime == null and g.mate == null and g.nodes == null and g.wtime == null) {
                    depth = 4;
                }
                // Threads from setoption

                const limits = core.search.SearchLimits{ .depth = depth, .nodes = nodes, .threads = threads_use };
                // Handle searchmoves filtering stub: if set, we will filter after search (small)
                var res = core.search.searchWithCancellation(session.board, limits, .{ .cancelled = &struct { var dummy = std.atomic.Value(bool).init(false); } .dummy });

                // Small stub: filter bestmove to searchmoves if provided and bestmove not in list
                if (g.searchmoves) |sm| {
                    if (res.bestmove) |bm| {
                        var buf: [5]u8 = undefined;
                        const uci = bm.toUci(&buf);
                        var found = false;
                        for (sm) |m| if (std.mem.eql(u8, m, uci)) { found = true; break; };
                        if (!found and sm.len > 0) {
                            // Pick first legal searchmove instead
                            if (core.move.Move.fromUci(sm[0])) |fm| {
                                var list = core.MoveList{};
                                core.movegen.generateLegal(session.board, &list);
                                for (list.moves[0..list.len]) |lm| {
                                    if (lm.from == fm.from and lm.to == fm.to and lm.promotion == fm.promotion) {
                                        res.bestmove = lm;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                // Handle ponder
                const ponder_str: []const u8 = if (g.ponder) " ponder" else "";

                if (res.bestmove) |bm| {
                    var buf: [5]u8 = undefined;
                    const uci = bm.toUci(&buf);
                    if (res.from_book) {
                        try stdout_w.interface.print("info string book hit {s}\n", .{uci});
                    } else {
                        const ph = core.phase.classify(session.board);
                        try stdout_w.interface.print("info string phase {s}{s}\n", .{ @tagName(ph), ponder_str });
                    }
                    // Also publish search info
                    try stdout_w.interface.print("info depth {d} score cp {d} nodes {d} pv {s}\n", .{ res.depth, res.score, res.nodes, uci });
                    try proto.events.publishBestmove(io, &stdout_w, uci);
                    if (g.ponder and res.bestmove != null) {
                        // Small ponder stub: no ponder move yet
                    }
                } else {
                    try stdout_w.interface.print("bestmove 0000\n", .{});
                    try stdout_w.interface.flush();
                }
                _ = g.binc;
                _ = g.winc;
            },
            .unknown => |s| {
                try stderr_w.interface.print("unknown command: {s}\n", .{s});
                try stderr_w.interface.flush();
            },
        }
    }
}

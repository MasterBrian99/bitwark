const std = @import("std");
const core = @import("bitwark_core");
const proto = @import("bitwark_protocol");

pub fn main(init: std.process.Init) !void {
    const arena = init.arena.allocator();
    const io = init.io;

    var stdout_buf: [8192]u8 = undefined;
    var stdout_w: std.Io.File.Writer = .init(.stdout(), io, &stdout_buf);
    var stderr_buf: [1024]u8 = undefined;
    var stderr_w: std.Io.File.Writer = .init(.stderr(), io, &stderr_buf);

    var session = proto.session.Session.init(core.Board.startingPosition());
    var controller = proto.controller.Controller{};
    const tt_alloc = std.heap.page_allocator;
    var engine = core.engine.Engine.init(tt_alloc, .{}) catch {
        try stderr_w.interface.print("error: TT alloc failed\n", .{});
        try stderr_w.interface.flush();
        return;
    };
    defer engine.deinit();

    var debug_on = false;
    _ = &debug_on;

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
        try stderr_w.interface.flush();

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
                // Gracefully accept known-unsupported options for GUI compat
                if (std.mem.eql(u8, opt.name, "Ponder") or std.mem.eql(u8, opt.name, "UCI_Chess960") or std.mem.eql(u8, opt.name, "MultiPV")) {
                    try proto.events.publishInfoString(io, &stdout_w, "option ok");
                    continue;
                }
                // Special handling for Clear Hash button: clear TT and report done
                if (std.mem.eql(u8, opt.name, "Clear Hash")) {
                    engine.clearHash();
                    try proto.events.publishInfoString(io, &stdout_w, "Clear Hash done");
                    continue;
                }
                // SyzygyPath has special success message with probing note
                const is_syzygy_path = std.mem.eql(u8, opt.name, "SyzygyPath");
                engine.setOption(opt.name, opt.value) catch |err| {
                    switch (err) {
                        error.UnknownOption => {
                            var buf: [256]u8 = undefined;
                            const msg = std.fmt.bufPrint(&buf, "error: unknown option {s}", .{opt.name}) catch "error: unknown option";
                            try proto.events.publishInfoString(io, &stdout_w, msg);
                        },
                        error.MissingValue => {
                            var buf: [256]u8 = undefined;
                            const msg = std.fmt.bufPrint(&buf, "error: missing value for {s}", .{opt.name}) catch "error: missing value";
                            try proto.events.publishInfoString(io, &stdout_w, msg);
                        },
                        error.InvalidValue => {
                            if (std.mem.eql(u8, opt.name, "Threads")) {
                                try proto.events.publishInfoString(io, &stdout_w, "error: Threads 1..16");
                            } else if (std.mem.eql(u8, opt.name, "Hash")) {
                                try proto.events.publishInfoString(io, &stdout_w, "error: Hash 1..1024");
                            } else if (std.mem.eql(u8, opt.name, "MoveOverhead")) {
                                try proto.events.publishInfoString(io, &stdout_w, "error: MoveOverhead 0..5000");
                            } else if (std.mem.eql(u8, opt.name, "SyzygyProbeDepth")) {
                                try proto.events.publishInfoString(io, &stdout_w, "error: SyzygyProbeDepth 1..100");
                            } else if (std.mem.eql(u8, opt.name, "SyzygyProbeLimit")) {
                                try proto.events.publishInfoString(io, &stdout_w, "error: SyzygyProbeLimit 0..7");
                            } else if (std.mem.eql(u8, opt.name, "OwnBook")) {
                                try proto.events.publishInfoString(io, &stdout_w, "error: OwnBook true/false");
                            } else if (std.mem.eql(u8, opt.name, "Syzygy50MoveRule")) {
                                try proto.events.publishInfoString(io, &stdout_w, "error: Syzygy50MoveRule true/false");
                            } else {
                                var buf: [256]u8 = undefined;
                                const msg = std.fmt.bufPrint(&buf, "error: invalid value for {s}", .{opt.name}) catch "error: invalid value";
                                try proto.events.publishInfoString(io, &stdout_w, msg);
                            }
                        },
                        else => {
                            try proto.events.publishInfoString(io, &stdout_w, "error: Hash alloc failed");
                        },
                    }
                    continue;
                };
                if (is_syzygy_path) {
                    if (opt.value) |v| {
                        const trimmed_v = std.mem.trim(u8, v, &std.ascii.whitespace);
                        if (trimmed_v.len > 0) {
                            try stdout_w.interface.print("info string SyzygyPath set \"{s}\" (probing not yet implemented)\n", .{trimmed_v});
                        } else {
                            try stdout_w.interface.print("info string SyzygyPath cleared (probing not yet implemented)\n", .{});
                        }
                    } else {
                        try stdout_w.interface.print("info string SyzygyPath cleared (probing not yet implemented)\n", .{});
                    }
                    try stdout_w.interface.flush();
                } else {
                    var buf: [256]u8 = undefined;
                    const msg = std.fmt.bufPrint(&buf, "{s} set", .{opt.name}) catch opt.name;
                    try proto.events.publishInfoString(io, &stdout_w, msg);
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
                engine.newGame();
            },
            .quit => break,
            .stop => {
                if (controller.isBusy()) controller.endSearch();
            },
            .ponderhit => {
                try proto.events.publishInfoString(io, &stdout_w, "ponderhit received");
            },
            .position_startpos => |p| {
                try stderr_w.interface.flush();
                const ok = session.setPosition(.startpos, null, p.moves) catch |err| {
                    try stderr_w.interface.print("error: position: {t}\n", .{err});
                    try stderr_w.interface.flush();
                    continue;
                };
                _ = ok;
                try stderr_w.interface.flush();
            },
            .position_fen => |p| {
                const ok = session.setPosition(.fen, p.fen, p.moves) catch |err| {
                    try stderr_w.interface.print("error: position: {t}\n", .{err});
                    try stderr_w.interface.flush();
                    continue;
                };
                _ = ok;
            },
            .perft => |p| {
                if (p.depth == 0 or p.depth > 7) {
                    try stdout_w.interface.print("info string error: perft requires depth 1..7\n", .{});
                    try stdout_w.interface.flush();
                    continue;
                }
                if (p.divide) {
                    var buf: [256]core.perft.DivideEntry = undefined;
                    const n = core.perft.perftDivide(session.board, p.depth, &buf);
                    var total: u64 = 0;
                    for (buf[0..n]) |e| {
                        const uci = e.uci[0..e.len];
                        try stdout_w.interface.print("{s}: {d}\n", .{ uci, e.nodes });
                        total += e.nodes;
                    }
                    try stdout_w.interface.print("\n{d}\n", .{total});
                    try stdout_w.interface.flush();
                } else {
                    const nodes = core.perft.perft(session.board, p.depth);
                    try stdout_w.interface.print("{d}\n", .{nodes});
                    try stdout_w.interface.flush();
                }
            },
            .bench => |b| {
                const depth: u8 = b.depth orelse 4;
                if (depth < 1 or depth > 12) {
                    try stdout_w.interface.print("info string error: bench requires depth 1..12\n", .{});
                    try stdout_w.interface.flush();
                    continue;
                }
                var out: [6]core.bench.BenchResult = undefined;
                const limits = core.search.SearchLimits{ .depth = depth, .threads = engine.config.threads, .use_book = false };
                const n = core.bench.runSuiteWithIo(io, limits, &out);
                var total_nodes: u64 = 0;
                var total_qnodes: u64 = 0;
                var total_cutoffs: u64 = 0;
                var total_ms: u64 = 0;
                for (out[0..n]) |r| {
                    var best_tmp: [5]u8 = undefined;
                    const best_src = r.bestUci();
                    @memcpy(best_tmp[0..best_src.len], best_src);
                    const best_uci = best_tmp[0..best_src.len];
                    const book_str: []const u8 = if (r.from_book) " book" else "";
                    try stdout_w.interface.print("info string bench {s} depth {d} nodes {d} qnodes {d} cutoffs {d} seldepth {d} time {d} nps {d} score {d} bestmove {s} phase {s}{s}\n", .{ r.name, r.depth, r.nodes, r.qnodes, r.beta_cutoffs, r.seldepth, r.time_ms, r.nps, r.score, best_uci, @tagName(r.phase), book_str });
                    total_nodes += r.nodes;
                    total_qnodes += r.qnodes;
                    total_cutoffs += r.beta_cutoffs;
                    total_ms += r.time_ms;
                }
                const total_nps: u64 = if (total_ms > 0) total_nodes * 1000 / total_ms else total_nodes;
                try stdout_w.interface.print("info string bench total nodes {d} qnodes {d} cutoffs {d} time {d} nps {d} threads {d} hash {d}\n", .{ total_nodes, total_qnodes, total_cutoffs, total_ms, total_nps, engine.config.threads, engine.config.hash_mb });
                try stdout_w.interface.flush();
            },
            .display => {
                try core.display.writeBoard(&stdout_w.interface, session.board);
                try stdout_w.interface.flush();
            },
            .eval => {
                const bd = core.eval.evaluate(session.board);
                try stdout_w.interface.print("info string eval {d} white / {d} stm\n", .{ bd.total(), core.eval.evaluateForSide(session.board) });
                for (core.eval.term_names, bd.toArray()) |name, val| {
                    try stdout_w.interface.print("info string eval {s:20} {d:5}\n", .{ name, val });
                }
                try stdout_w.interface.flush();
            },
            .compiler => {
                try core.compiler.writeCompilerInfo(&stdout_w.interface);
                try stdout_w.interface.flush();
            },
            .speedtest => |s| {
                const cur_threads = engine.config.threads;
                const cur_hash = engine.config.hash_mb;
                const thr_opt = s.threads;
                const hmb_opt = s.hash;
                const secs_opt = s.secs;
                var thr: u8 = cur_threads;
                var hmb: u32 = cur_hash;
                var secs: u32 = 5;
                if (thr_opt == null and hmb_opt == null and secs_opt == null) {
                    // use defaults (current threads/hash, 5s)
                } else {
                    if (thr_opt == null or hmb_opt == null or secs_opt == null) {
                        try stdout_w.interface.print("info string error: speedtest requires <Threads 1..16> <Hash 1..1024> <Seconds 1..60>\n", .{});
                        try stdout_w.interface.flush();
                        continue;
                    }
                    thr = thr_opt.?;
                    hmb = hmb_opt.?;
                    secs = secs_opt.?;
                    if (thr < 1 or thr > 16) {
                        try stdout_w.interface.print("info string error: speedtest Threads 1..16\n", .{});
                        try stdout_w.interface.flush();
                        continue;
                    }
                    if (hmb < 1 or hmb > 1024) {
                        try stdout_w.interface.print("info string error: speedtest Hash 1..1024\n", .{});
                        try stdout_w.interface.flush();
                        continue;
                    }
                    if (secs < 1 or secs > 60) {
                        try stdout_w.interface.print("info string error: speedtest Seconds 1..60\n", .{});
                        try stdout_w.interface.flush();
                        continue;
                    }
                    // Apply permanently through setOption path so config stays consistent
                    var tbuf: [16]u8 = undefined;
                    var hbuf: [16]u8 = undefined;
                    const tstr = std.fmt.bufPrint(&tbuf, "{d}", .{thr}) catch "1";
                    const hstr = std.fmt.bufPrint(&hbuf, "{d}", .{hmb}) catch "16";
                    engine.setOption("Threads", tstr) catch {
                        try stdout_w.interface.print("info string error: speedtest Threads invalid\n", .{});
                        try stdout_w.interface.flush();
                        continue;
                    };
                    engine.setOption("Hash", hstr) catch |err| {
                        if (err == error.InvalidValue) {
                            try stdout_w.interface.print("info string error: speedtest Hash invalid\n", .{});
                        } else {
                            try stdout_w.interface.print("info string error: speedtest Hash alloc failed\n", .{});
                        }
                        try stdout_w.interface.flush();
                        continue;
                    };
                }
                // Fixed-position harness: run startpos at depth 4 repeatedly until secs elapsed
                const start = std.Io.Clock.Timestamp.now(io, .awake);
                const deadline_ns: u64 = @as(u64, secs) * 1_000_000_000;
                var total_nodes: u64 = 0;
                var total_qnodes: u64 = 0;
                var iterations: u64 = 0;
                const board = core.fen.parseFen(core.bench.suite[0].fen) catch session.board;
                while (true) {
                    const elapsed_check = start.untilNow(io).raw.nanoseconds;
                    if (elapsed_check >= @as(i96, deadline_ns)) break;
                    var dummy = std.atomic.Value(bool).init(false);
                    const tok = core.search.CancellationToken{ .cancelled = &dummy };
                    const limits = core.search.SearchLimits{ .depth = 4, .threads = engine.config.threads, .use_book = false };
                    const res = core.search.searchWithCancellation(board, limits, tok);
                    total_nodes += res.nodes;
                    total_qnodes += res.qnodes;
                    iterations += 1;
                    if (iterations > 1000) break;
                }
                const elapsed_ns_i96 = start.untilNow(io).raw.nanoseconds;
                const elapsed_ns: u64 = if (elapsed_ns_i96 > 0) @intCast(elapsed_ns_i96) else 1;
                const elapsed_ms: u64 = @max(1, elapsed_ns / 1_000_000);
                const nps: u64 = if (elapsed_ns > 0) total_nodes * 1_000_000_000 / elapsed_ns else total_nodes;
                try stdout_w.interface.print("info string speedtest threads {d} hash {d} secs {d} iterations {d} nodes {d} qnodes {d} time {d} nps {d}\n", .{ engine.config.threads, engine.config.hash_mb, secs, iterations, total_nodes, total_qnodes, elapsed_ms, nps });
                try stdout_w.interface.flush();
            },
            .go => |g| {
                if (!controller.startSearch()) {
                    try stderr_w.interface.print("info string busy\n", .{});
                    try stderr_w.interface.flush();
                    continue;
                }
                defer controller.endSearch();

                var depth: u8 = 3;
                const nodes = g.nodes;
                const threads_use = engine.config.threads;
                const use_book = engine.config.use_opening_book;

                if (g.depth) |d| depth = d;
                if (g.movetime) |ms| {
                    depth = if (ms < 50) 2 else if (ms < 200) 3 else if (ms < 1000) 4 else 5;
                }
                if (g.wtime != null or g.btime != null) {
                    if (depth == 3 and g.depth == null and g.movetime == null and g.mate == null and g.infinite == false and g.nodes == null) {
                        const stm = session.board.side_to_move;
                        const my_time = if (stm == .white) g.wtime else g.btime;
                        const my_inc = if (stm == .white) g.winc else g.binc;
                        if (my_time) |t| {
                            if (t < 1000) depth = 2 else if (t < 5000) depth = 3 else depth = 4;
                            _ = my_inc;
                        }
                    }
                    _ = g.movestogo;
                }
                if (g.mate) |m| {
                    depth = @min(m * 2, 6);
                }
                if (g.infinite and g.depth == null and g.movetime == null and g.mate == null and g.nodes == null and g.wtime == null) {
                    depth = 4;
                }

                const limits = core.search.SearchLimits{ .depth = depth, .nodes = nodes, .threads = threads_use, .use_book = use_book };
                var res = core.search.searchWithCancellation(session.board, limits, .{ .cancelled = &struct {
                    var dummy = std.atomic.Value(bool).init(false);
                }.dummy });

                if (g.searchmoves) |sm| {
                    if (res.bestmove) |bm| {
                        var buf: [5]u8 = undefined;
                        const uci = bm.toUci(&buf);
                        var found = false;
                        for (sm) |m| if (std.mem.eql(u8, m, uci)) {
                            found = true;
                            break;
                        };
                        if (!found and sm.len > 0) {
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
                    try stdout_w.interface.print("info depth {d} seldepth {d} score cp {d} nodes {d} qnodes {d} cutoffs {d} nps {d} pv {s}\n", .{ res.depth, res.seldepth, res.score, res.nodes, res.qnodes, res.beta_cutoffs, 0, uci });
                    try proto.events.publishBestmove(io, &stdout_w, uci);
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
        try stderr_w.interface.flush();
    }
}

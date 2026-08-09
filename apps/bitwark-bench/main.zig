const std = @import("std");
const core = @import("bitwark_core");

const VERSION = "0.1.0";

const usage =
    \\bitwark-bench — unified search benchmark (shared suite, real timing)
    \\
    \\Usage: bitwark-bench [options]
    \\       bitwark-bench --depth 4 [--suite] [--fen <FEN>] [--nobook] [--nodes <N>]
    \\
    \\Options:
    \\  --fen <FEN>           Single FEN position (default: startpos, ignored with --suite)
    \\  --depth <N>           Search depth (default: 4, 1..12)
    \\  --nodes <N>           Optional node limit (single position only)
    \\  --threads <N>         Threads for parallel root (default: 1, 1..16)
    \\  --suite               Run 6-position fixed suite (ignores --fen/--nodes)
    \\  --nobook              Disable opening book (single position only; suite is always nobook)
    \\  --help                Show this help to stdout
    \\  --version             Show version to stdout
    \\
    \\Output (stdout):
    \\  depth <N> nodes <N> qnodes <N> cutoffs <N> seldepth <N> nps <N> time <ms> score <cp> bestmove <uci> phase <opening|middlegame|endgame> threads <N>
    \\  suite line: bench <name> depth <d> nodes <n> qnodes <n> cutoffs <n> seldepth <n> time <ms> nps <n> score <cp> bestmove <uci> phase <s>
    \\
    \\Exit codes:
    \\  0 success, 1 usage error, 2 FEN error
    \\
;

fn printHelp(io: std.Io, writer: *std.Io.File.Writer) !void {
    try writer.interface.writeAll(usage);
    try writer.interface.flush();
    _ = io;
}

fn printVersion(io: std.Io, writer: *std.Io.File.Writer) !void {
    try writer.interface.print("bitwark-bench {s}\n", .{VERSION});
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

    var fen_opt: ?[]const u8 = null;
    var depth: u8 = 4;
    var nodes_limit: ?u64 = null;
    var threads: u8 = 1;
    var nobook = false;
    var suite_mode = false;

    var i: usize = 1;
    while (i < args.len) : (i += 1) {
        const a = args[i];
        if (std.mem.eql(u8, a, "--help") or std.mem.eql(u8, a, "-h")) {
            try printHelp(io, &stdout_w);
            return;
        } else if (std.mem.eql(u8, a, "--version") or std.mem.eql(u8, a, "-V")) {
            try printVersion(io, &stdout_w);
            return;
        } else if (std.mem.eql(u8, a, "--suite")) {
            suite_mode = true;
        } else if (std.mem.eql(u8, a, "--nobook")) {
            nobook = true;
        } else if (std.mem.eql(u8, a, "--fen")) {
            i += 1;
            if (i >= args.len) {
                try stderr_w.interface.print("missing --fen value\n{s}", .{usage});
                try stderr_w.interface.flush();
                std.process.exit(1);
            }
            fen_opt = args[i];
        } else if (std.mem.eql(u8, a, "--depth")) {
            i += 1;
            if (i >= args.len) {
                try stderr_w.interface.print("missing --depth value\n{s}", .{usage});
                try stderr_w.interface.flush();
                std.process.exit(1);
            }
            depth = std.fmt.parseInt(u8, args[i], 10) catch {
                try stderr_w.interface.print("invalid --depth\n{s}", .{usage});
                try stderr_w.interface.flush();
                std.process.exit(1);
            };
            if (depth < 1 or depth > 12) {
                try stderr_w.interface.print("depth 1..12\n{s}", .{usage});
                try stderr_w.interface.flush();
                std.process.exit(1);
            }
        } else if (std.mem.eql(u8, a, "--nodes")) {
            i += 1;
            if (i >= args.len) {
                try stderr_w.interface.print("missing --nodes value\n{s}", .{usage});
                try stderr_w.interface.flush();
                std.process.exit(1);
            }
            nodes_limit = std.fmt.parseInt(u64, args[i], 10) catch {
                try stderr_w.interface.print("invalid --nodes\n{s}", .{usage});
                try stderr_w.interface.flush();
                std.process.exit(1);
            };
        } else if (std.mem.eql(u8, a, "--threads")) {
            i += 1;
            if (i >= args.len) {
                try stderr_w.interface.print("missing --threads value\n{s}", .{usage});
                try stderr_w.interface.flush();
                std.process.exit(1);
            }
            threads = std.fmt.parseInt(u8, args[i], 10) catch {
                try stderr_w.interface.print("invalid --threads\n{s}", .{usage});
                try stderr_w.interface.flush();
                std.process.exit(1);
            };
            if (threads < 1 or threads > 16) {
                try stderr_w.interface.print("threads 1..16\n{s}", .{usage});
                try stderr_w.interface.flush();
                std.process.exit(1);
            }
        } else {
            try stderr_w.interface.print("unknown option: {s}\n{s}", .{ a, usage });
            try stderr_w.interface.flush();
            std.process.exit(1);
        }
    }

    // Build shared config from CLI flags (threads + nobook map into EngineConfig)
    const cfg = core.config.EngineConfig{ .threads = threads, .use_opening_book = !nobook };

    if (suite_mode) {
        var out: [6]core.bench.BenchResult = undefined;
        const limits = core.search.SearchLimits{ .depth = depth, .threads = cfg.threads, .use_book = false };
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
            try stdout_w.interface.print("bench {s} depth {d} nodes {d} qnodes {d} cutoffs {d} seldepth {d} time {d} nps {d} score {d} bestmove {s} phase {s}{s}\n", .{ r.name, r.depth, r.nodes, r.qnodes, r.beta_cutoffs, r.seldepth, r.time_ms, r.nps, r.score, best_uci, @tagName(r.phase), book_str });
            total_nodes += r.nodes;
            total_qnodes += r.qnodes;
            total_cutoffs += r.beta_cutoffs;
            total_ms += r.time_ms;
        }
        const total_nps: u64 = if (total_ms > 0) total_nodes * 1000 / total_ms else total_nodes;
        try stdout_w.interface.print("total nodes {d} qnodes {d} cutoffs {d} time {d} nps {d} threads {d}\n", .{ total_nodes, total_qnodes, total_cutoffs, total_ms, total_nps, cfg.threads });
        try stdout_w.interface.flush();
        return;
    }

    var board: core.Board = undefined;
    if (fen_opt) |f| {
        board = core.fen.parseFen(f) catch {
            try stderr_w.interface.print("invalid FEN\n", .{});
            try stderr_w.interface.flush();
            std.process.exit(2);
        };
    } else {
        board = core.Board.startingPosition();
    }

    const phase = core.phase.classify(board);
    const limits = core.search.SearchLimits{ .depth = depth, .nodes = nodes_limit, .threads = cfg.threads, .use_book = cfg.use_opening_book };
    const start = std.Io.Clock.Timestamp.now(io, .awake);
    var dummy = std.atomic.Value(bool).init(false);
    const tok = core.search.CancellationToken{ .cancelled = &dummy };
    const res = core.search.searchWithCancellation(board, limits, tok);
    const elapsed_ns_i96 = start.untilNow(io).raw.nanoseconds;
    const elapsed_ns: u64 = if (elapsed_ns_i96 > 0) @intCast(elapsed_ns_i96) else 1;
    const elapsed_ms: u64 = @max(1, elapsed_ns / 1_000_000);
    const nps: u64 = if (elapsed_ns > 0) res.nodes * 1_000_000_000 / elapsed_ns else res.nodes;

    var best_str: [5]u8 = undefined;
    const best_uci = if (res.bestmove) |bm| bm.toUci(&best_str) else "0000";
    const book_str: []const u8 = if (res.from_book) " book" else "";

    try stdout_w.interface.print("depth {d} nodes {d} qnodes {d} cutoffs {d} seldepth {d} nps {d} time {d} score {d} bestmove {s} phase {s} threads {d}{s}\n", .{ res.depth, res.nodes, res.qnodes, res.beta_cutoffs, res.seldepth, nps, elapsed_ms, res.score, best_uci, @tagName(phase), cfg.threads, book_str });
    try stdout_w.interface.flush();
}

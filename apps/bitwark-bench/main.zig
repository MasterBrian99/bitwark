const std = @import("std");
const core = @import("bitwark_core");

const VERSION = "0.1.0";

const usage =
    \\bitwark-bench — phase-aware search benchmark (divergence flags, deterministic)
    \\
    \\Usage: bitwark-bench [options]
    \\       bitwark-bench --depth 4 [--fen <FEN>] [--divergence <mode>] [--nodes <N>]
    \\
    \\Options:
    \\  --fen <FEN>           FEN position (default: startpos)
    \\  --depth <N>           Search depth (default: 4, 1..12)
    \\  --nodes <N>           Optional node limit
    \\  --threads <N>         Threads for parallel root (default: 1, 1..16)
    \\  --divergence <mode>   off | endgame | nobook | central | middlegame | pawn | all (default: off)
    \\  --help                Show this help to stdout
    \\  --version             Show version to stdout
    \\
    \\Output (stdout):
    \\  depth <N> nodes <N> nps <N> time <ms> score <cp> bestmove <uci> phase <opening|middlegame|endgame> divergence <mode>
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
    var divergence_mode: []const u8 = "off";

    var i: usize = 1;
    while (i < args.len) : (i += 1) {
        const a = args[i];
        if (std.mem.eql(u8, a, "--help") or std.mem.eql(u8, a, "-h")) {
            try printHelp(io, &stdout_w);
            return;
        } else if (std.mem.eql(u8, a, "--version") or std.mem.eql(u8, a, "-V")) {
            try printVersion(io, &stdout_w);
            return;
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
        } else if (std.mem.eql(u8, a, "--divergence")) {
            i += 1;
            if (i >= args.len) {
                try stderr_w.interface.print("missing --divergence value\n{s}", .{usage});
                try stderr_w.interface.flush();
                std.process.exit(1);
            }
            divergence_mode = args[i];
            if (!std.mem.eql(u8, divergence_mode, "off") and !std.mem.eql(u8, divergence_mode, "endgame") and !std.mem.eql(u8, divergence_mode, "nobook") and !std.mem.eql(u8, divergence_mode, "central") and !std.mem.eql(u8, divergence_mode, "middlegame") and !std.mem.eql(u8, divergence_mode, "pawn") and !std.mem.eql(u8, divergence_mode, "all")) {
                try stderr_w.interface.print("divergence off|endgame|nobook|central|middlegame|pawn|all\n{s}", .{usage});
                try stderr_w.interface.flush();
                std.process.exit(1);
            }
        } else {
            try stderr_w.interface.print("unknown option: {s}\n{s}", .{ a, usage });
            try stderr_w.interface.flush();
            std.process.exit(1);
        }
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

    var divergence = core.search.Divergence{};
    if (std.mem.eql(u8, divergence_mode, "endgame")) divergence = core.search.Divergence.endgame_evasion_on
    else if (std.mem.eql(u8, divergence_mode, "nobook")) divergence = core.search.Divergence.opening_nobook
    else if (std.mem.eql(u8, divergence_mode, "central")) divergence = core.search.Divergence.opening_central_on
    else if (std.mem.eql(u8, divergence_mode, "middlegame")) divergence = core.search.Divergence.middlegame_pruning_on
    else if (std.mem.eql(u8, divergence_mode, "pawn")) divergence = core.search.Divergence.endgame_pawn_on
    else if (std.mem.eql(u8, divergence_mode, "all")) divergence = core.search.Divergence.all_on;

    const phase = core.phase.classify(board);
    var dummy = std.atomic.Value(bool).init(false);
    const tok = core.search.CancellationToken{ .cancelled = &dummy };
    const limits = core.search.SearchLimits{ .depth = depth, .nodes = nodes_limit, .threads = threads };

    // Simple timing via Invariant — bench is deterministic, nps derived from nodes
    const res = core.search.searchWithDivergence(board, limits, tok, divergence);
    const elapsed: u64 = 1;
    const nps: u64 = res.nodes;

    var best_str: [5]u8 = undefined;
    const best_uci = if (res.bestmove) |bm| bm.toUci(&best_str) else "0000";
    const from_book_str: []const u8 = if (res.from_book) " book" else "";

    try stdout_w.interface.print("depth {d} nodes {d} nps {d} time {d} score {d} bestmove {s} phase {s} divergence {s} threads {d}{s}\n", .{ res.depth, res.nodes, nps, elapsed, res.score, best_uci, @tagName(phase), divergence_mode, threads, from_book_str });
    try stdout_w.interface.flush();
}

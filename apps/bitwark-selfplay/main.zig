const std = @import("std");
const core = @import("bitwark_core");

const VERSION = "0.1.0";

const usage =
    \\bitwark-selfplay — regression self-play (phase-aware search, divergence flags)
    \\
    \\Usage: bitwark-selfplay [options]
    \\       bitwark-selfplay --depth 3 [--fen <FEN>] [--moves 60] [--divergence endgame]
    \\
    \\Options:
    \\  --fen <FEN>           Start FEN (default: startpos)
    \\  --depth <N>           Search depth per ply (default: 3, 1..6)
    \\  --moves <N>           Max plies (default: 60)
    \\  --divergence <mode>   off | endgame | nobook | central | all (default: off) 
    \\  --help                Show this help to stdout
    \\  --version             Show version to stdout
    \\
    \\Output (stdout, diagnostics to stderr):
    \\  One UCI move per line, final line `result <1-0|0-1|1/2-1/2|*>`
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
    try writer.interface.print("bitwark-selfplay {s}\n", .{VERSION});
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
    var depth: u8 = 3;
    var max_plies: usize = 60;
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
            if (depth < 1 or depth > 6) {
                try stderr_w.interface.print("depth 1..6\n{s}", .{usage});
                try stderr_w.interface.flush();
                std.process.exit(1);
            }
        } else if (std.mem.eql(u8, a, "--moves")) {
            i += 1;
            if (i >= args.len) {
                try stderr_w.interface.print("missing --moves value\n{s}", .{usage});
                try stderr_w.interface.flush();
                std.process.exit(1);
            }
            max_plies = std.fmt.parseInt(usize, args[i], 10) catch {
                try stderr_w.interface.print("invalid --moves\n{s}", .{usage});
                try stderr_w.interface.flush();
                std.process.exit(1);
            };
        } else if (std.mem.eql(u8, a, "--divergence")) {
            i += 1;
            if (i >= args.len) {
                try stderr_w.interface.print("missing --divergence value\n{s}", .{usage});
                try stderr_w.interface.flush();
                std.process.exit(1);
            }
            divergence_mode = args[i];
            if (!std.mem.eql(u8, divergence_mode, "off") and !std.mem.eql(u8, divergence_mode, "endgame") and !std.mem.eql(u8, divergence_mode, "nobook") and !std.mem.eql(u8, divergence_mode, "central") and !std.mem.eql(u8, divergence_mode, "all")) {
                try stderr_w.interface.print("divergence off|endgame|nobook|central|all\n{s}", .{usage});
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
    else if (std.mem.eql(u8, divergence_mode, "all")) divergence = .{ .opening_book = true, .endgame_check_evasion_qsearch = true, .opening_central_bonus = true };

    var plies: usize = 0;
    while (plies < max_plies) : (plies += 1) {
        var list = core.MoveList{};
        core.movegen.generateLegal(board, &list);
        if (list.len == 0) {
            const in_check = core.attacks.isSquareAttacked(board, board.kingSquare(board.side_to_move).?, board.side_to_move.opposite());
            const result: []const u8 = if (in_check) (if (board.side_to_move == .white) "0-1" else "1-0") else "1/2-1/2";
            try stdout_w.interface.print("result {s}\n", .{result});
            try stdout_w.interface.flush();
            try stderr_w.interface.print("game over ply {d} result {s} phase {s}\n", .{ plies, result, @tagName(core.phase.classify(board)) });
            try stderr_w.interface.flush();
            return;
        }
        var dummy: bool = false;
        const tok = core.search.CancellationToken{ .cancelled = &dummy };
        const limits = core.search.SearchLimits{ .depth = depth };
        const res = core.search.searchWithDivergence(board, limits, tok, divergence);
        const bm = res.bestmove orelse {
            try stdout_w.interface.print("result *\n", .{});
            try stdout_w.interface.flush();
            return;
        };
        var buf: [5]u8 = undefined;
        const uci = bm.toUci(&buf);
        try stdout_w.interface.print("{s}\n", .{uci});
        // keep stdout line buffered but flush each ply for scriptability
        try stdout_w.interface.flush();
        try stderr_w.interface.print("ply {d} {s} phase {s} nodes {d} score {d}\n", .{ plies + 1, uci, @tagName(core.phase.classify(board)), res.nodes, res.score });
        try stderr_w.interface.flush();
        core.movegen.applyMove(&board, bm);
    }
    try stdout_w.interface.print("result *\n", .{});
    try stdout_w.interface.flush();
    try stderr_w.interface.print("max plies {d} reached phase {s}\n", .{ max_plies, @tagName(core.phase.classify(board)) });
    try stderr_w.interface.flush();
}

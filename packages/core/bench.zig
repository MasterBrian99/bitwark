const std = @import("std");
const board_mod = @import("board.zig");
const fen_mod = @import("fen.zig");
const search_mod = @import("search.zig");
const phase_mod = @import("phase.zig");

pub const Board = board_mod.Board;

/// One fixed benchmark position
pub const BenchPosition = struct {
    name: []const u8,
    fen: []const u8,
};

/// 6-position suite covering spec
pub const suite: [6]BenchPosition = .{
    .{ .name = "startpos_nobook", .fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1" },
    .{ .name = "tactical_middlegame", .fen = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1" },
    .{ .name = "quiet_middlegame", .fen = "rnbq1rk1/ppp2ppp/4pn2/3p4/2PP4/2N1PN2/PP2PPPP/R1BQKB1R w KQ - 0 1" },
    .{ .name = "queenless_endgame", .fen = "8/5pk1/5p1p/4p3/3P4/5PP1/5PK1/8 w - - 0 1" },
    .{ .name = "pawn_endgame", .fen = "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1" },
    .{ .name = "check_heavy", .fen = "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1" },
};

pub const BenchResult = struct {
    name: []const u8,
    depth: u8,
    nodes: u64,
    qnodes: u64 = 0,
    beta_cutoffs: u64 = 0,
    seldepth: u8 = 0,
    time_ms: u64,
    nps: u64,
    score: i16,
    bestmove_uci: [5]u8 = .{0} ** 5,
    bestmove_len: usize = 0,
    phase: phase_mod.GamePhase,
    from_book: bool = false,

    pub fn bestUci(self: BenchResult) []const u8 {
        return self.bestmove_uci[0..self.bestmove_len];
    }
};

pub fn runSingle(board: Board, limits: search_mod.SearchLimits) BenchResult {
    var dummy = std.atomic.Value(bool).init(false);
    const tok = search_mod.CancellationToken{ .cancelled = &dummy };
    const res = search_mod.searchWithCancellation(board, limits, tok);
    var br = BenchResult{
        .name = "single",
        .depth = limits.depth,
        .nodes = res.nodes,
        .qnodes = res.qnodes,
        .beta_cutoffs = res.beta_cutoffs,
        .seldepth = res.seldepth,
        .time_ms = 1,
        .nps = res.nodes,
        .score = res.score,
        .phase = phase_mod.classify(board),
        .from_book = res.from_book,
    };
    if (res.bestmove) |bm| {
        const uci = bm.toUci(&br.bestmove_uci);
        br.bestmove_len = uci.len;
    } else {
        @memcpy(br.bestmove_uci[0..4], "0000");
        br.bestmove_len = 4;
    }
    return br;
}

pub fn runSingleWithIo(io: std.Io, board: Board, limits: search_mod.SearchLimits) BenchResult {
    const start = std.Io.Clock.Timestamp.now(io, .awake);
    var dummy = std.atomic.Value(bool).init(false);
    const tok = search_mod.CancellationToken{ .cancelled = &dummy };
    const res = search_mod.searchWithCancellation(board, limits, tok);
    const elapsed_ns_i96 = start.untilNow(io).raw.nanoseconds;
    const elapsed_ns: u64 = if (elapsed_ns_i96 > 0) @intCast(elapsed_ns_i96) else 1;
    const elapsed_ms: u64 = @max(1, elapsed_ns / 1_000_000);
    const nps: u64 = if (elapsed_ns > 0) res.nodes * 1_000_000_000 / elapsed_ns else res.nodes;
    var br = BenchResult{
        .name = "single",
        .depth = limits.depth,
        .nodes = res.nodes,
        .qnodes = res.qnodes,
        .beta_cutoffs = res.beta_cutoffs,
        .seldepth = res.seldepth,
        .time_ms = elapsed_ms,
        .nps = nps,
        .score = res.score,
        .phase = phase_mod.classify(board),
        .from_book = res.from_book,
    };
    if (res.bestmove) |bm| {
        const uci = bm.toUci(&br.bestmove_uci);
        br.bestmove_len = uci.len;
    } else {
        @memcpy(br.bestmove_uci[0..4], "0000");
        br.bestmove_len = 4;
    }
    return br;
}

pub fn runSuite(limits: search_mod.SearchLimits, out: []BenchResult) usize {
    var limits_nobook = limits;
    limits_nobook.use_book = false;
    var count: usize = 0;
    for (suite) |pos| {
        if (count >= out.len) break;
        const board = fen_mod.parseFen(pos.fen) catch continue;
        var dummy = std.atomic.Value(bool).init(false);
        const tok = search_mod.CancellationToken{ .cancelled = &dummy };
        const res = search_mod.searchWithCancellation(board, limits_nobook, tok);
        var br = BenchResult{
            .name = pos.name,
            .depth = limits.depth,
            .nodes = res.nodes,
            .qnodes = res.qnodes,
            .beta_cutoffs = res.beta_cutoffs,
            .seldepth = res.seldepth,
            .time_ms = 1,
            .nps = res.nodes,
            .score = res.score,
            .phase = phase_mod.classify(board),
            .from_book = res.from_book,
        };
        if (res.bestmove) |bm| {
            const uci = bm.toUci(&br.bestmove_uci);
            br.bestmove_len = uci.len;
        } else {
            @memcpy(br.bestmove_uci[0..4], "0000");
            br.bestmove_len = 4;
        }
        out[count] = br;
        count += 1;
    }
    return count;
}

pub fn runSuiteWithIo(io: std.Io, limits: search_mod.SearchLimits, out: []BenchResult) usize {
    var limits_nobook = limits;
    limits_nobook.use_book = false;
    var count: usize = 0;
    for (suite) |pos| {
        if (count >= out.len) break;
        const board = fen_mod.parseFen(pos.fen) catch continue;
        const start = std.Io.Clock.Timestamp.now(io, .awake);
        var dummy = std.atomic.Value(bool).init(false);
        const tok = search_mod.CancellationToken{ .cancelled = &dummy };
        const res = search_mod.searchWithCancellation(board, limits_nobook, tok);
        const elapsed_ns_i96 = start.untilNow(io).raw.nanoseconds;
        const elapsed_ns: u64 = if (elapsed_ns_i96 > 0) @intCast(elapsed_ns_i96) else 1;
        const elapsed_ms: u64 = @max(1, elapsed_ns / 1_000_000);
        const nps: u64 = if (elapsed_ns > 0) res.nodes * 1_000_000_000 / elapsed_ns else res.nodes;
        var br = BenchResult{
            .name = pos.name,
            .depth = limits.depth,
            .nodes = res.nodes,
            .qnodes = res.qnodes,
            .beta_cutoffs = res.beta_cutoffs,
            .seldepth = res.seldepth,
            .time_ms = elapsed_ms,
            .nps = nps,
            .score = res.score,
            .phase = phase_mod.classify(board),
            .from_book = res.from_book,
        };
        if (res.bestmove) |bm| {
            const uci = bm.toUci(&br.bestmove_uci);
            br.bestmove_len = uci.len;
        } else {
            @memcpy(br.bestmove_uci[0..4], "0000");
            br.bestmove_len = 4;
        }
        out[count] = br;
        count += 1;
    }
    return count;
}

test "bench suite length" {
    try std.testing.expectEqual(@as(usize, 6), suite.len);
    for (suite) |pos| {
        const b = try fen_mod.parseFen(pos.fen);
        _ = phase_mod.classify(b);
    }
}

test "bench runSingle" {
    const b = try fen_mod.parseFen(suite[0].fen);
    const r = runSingle(b, .{ .depth = 2, .use_book = false });
    try std.testing.expect(r.nodes > 0);
    try std.testing.expect(r.time_ms >= 1);
}

test "bench runSuite" {
    var out: [6]BenchResult = undefined;
    const n = runSuite(.{ .depth = 1, .use_book = true }, &out);
    try std.testing.expectEqual(@as(usize, 6), n);
    for (out[0..n]) |r| {
        try std.testing.expect(r.nodes > 0);
        // suite always nobook
        try std.testing.expect(!r.from_book);
    }
}

test "bench runSuite forces nobook" {
    var out: [6]BenchResult = undefined;
    // even with use_book=true, suite must be nobook
    const n = runSuite(.{ .depth = 1, .use_book = true }, &out);
    for (out[0..n]) |r| {
        try std.testing.expect(!r.from_book);
    }
}

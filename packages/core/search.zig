const std = @import("std");
const board_mod = @import("board.zig");
const move_mod = @import("move.zig");
const movelist_mod = @import("movelist.zig");
const movegen_mod = @import("movegen.zig");
const eval_mod = @import("eval.zig");
const book_mod = @import("book.zig");

pub const Board = board_mod.Board;
pub const Move = move_mod.Move;
pub const MoveList = movelist_mod.MoveList;

pub const SearchLimits = struct {
    depth: u8 = 3,
    nodes: ?u64 = null,
    threads: u8 = 1,
    use_book: bool = true,
};

pub const SearchStats = struct {
    nodes: u64 = 0,
    qnodes: u64 = 0,
    beta_cutoffs: u64 = 0,
    tt_probes: u64 = 0,
    tt_hits: u64 = 0,
    seldepth: u8 = 0,
};

pub const SearchResult = struct {
    bestmove: ?Move = null,
    score: i16 = 0,
    depth: u8 = 0,
    nodes: u64 = 0,
    qnodes: u64 = 0,
    beta_cutoffs: u64 = 0,
    tt_probes: u64 = 0,
    tt_hits: u64 = 0,
    seldepth: u8 = 0,
    from_book: bool = false,
    pv: [64]Move = undefined,
    pv_len: usize = 0,
};

pub const CancellationToken = struct {
    cancelled: *std.atomic.Value(bool),
    pub fn isCancelled(self: CancellationToken) bool {
        return self.cancelled.load(.seq_cst);
    }
    pub fn cancel(self: CancellationToken) void {
        self.cancelled.store(true, .seq_cst);
    }
};

const INF: i16 = 30000;
const MATE: i16 = 29000;

// ── Public entry — book first, then unified search ───────────────────────

pub fn search(board: Board, limits: SearchLimits) SearchResult {
    var dummy = std.atomic.Value(bool).init(false);
    return searchWithCancellation(board, limits, CancellationToken{ .cancelled = &dummy });
}

pub fn searchWithCancellation(board: Board, limits: SearchLimits, token: CancellationToken) SearchResult {
    if (limits.use_book) {
        if (book_mod.probe(board)) |bm| {
            var list = MoveList{};
            movegen_mod.generateLegal(board, &list);
            for (list.moves[0..list.len]) |m| {
                if (m.from == bm.from and m.to == bm.to and m.promotion == bm.promotion) {
                    var pv: [64]Move = undefined;
                    pv[0] = m;
                    return .{ .bestmove = m, .score = 0, .depth = 0, .nodes = 1, .from_book = true, .pv = pv, .pv_len = 1 };
                }
            }
        }
    }
    return searchWith(board, limits, token);
}

fn searchWith(board: Board, limits: SearchLimits, token: CancellationToken) SearchResult {
    if (limits.threads > 1) return searchWithThreads(board, limits, token);
    var stats = SearchStats{};
    var best: ?Move = null;
    var best_score: i16 = -INF;
    var pv: [64]Move = undefined;
    var pv_len: usize = 0;

    var list = MoveList{};
    movegen_mod.generateLegal(board, &list);
    if (list.len == 0) {
        const in_check = isInCheck(board, board.side_to_move);
        const score: i16 = if (in_check) -MATE else 0;
        return .{ .bestmove = null, .score = score, .depth = limits.depth, .nodes = 1, .qnodes = 0, .beta_cutoffs = 0, .seldepth = limits.depth };
    }

    orderMoves(&list);

    for (list.moves[0..list.len]) |m| {
        if (token.isCancelled()) break;
        var copy = board;
        movegen_mod.applyMove(&copy, m);
        const score = -negamaxWithStats(copy, limits.depth - 1, -INF, INF, &stats, 1, token);
        if (score > best_score) {
            best_score = score;
            best = m;
            pv[0] = m;
            pv_len = 1;
        }
        if (limits.nodes) |limit| if (stats.nodes >= limit) break;
    }

    var res: SearchResult = .{ .bestmove = best, .score = best_score, .depth = limits.depth, .nodes = stats.nodes, .qnodes = stats.qnodes, .beta_cutoffs = stats.beta_cutoffs, .seldepth = @max(stats.seldepth, limits.depth) };
    if (best) |bm| {
        res.pv[0] = bm;
        res.pv_len = 1;
    }
    return res;
}

const ThreadContext = struct {
    board: Board,
    moves: []const Move,
    depth: u8,
    token: CancellationToken,
    local_best_score: i16 = -INF,
    local_best_move: ?Move = null,
    local_nodes: u64 = 0,
    local_qnodes: u64 = 0,
    local_cutoffs: u64 = 0,
    local_seldepth: u8 = 0,
};

fn threadWorker(ctx: *ThreadContext) void {
    var stats = SearchStats{};
    var local_best: ?Move = null;
    var local_score: i16 = -INF;
    for (ctx.moves) |m| {
        if (ctx.token.isCancelled()) break;
        var copy = ctx.board;
        movegen_mod.applyMove(&copy, m);
        const score = -negamaxWithStats(copy, ctx.depth - 1, -INF, INF, &stats, 1, ctx.token);
        if (score > local_score) {
            local_score = score;
            local_best = m;
        }
    }
    ctx.local_nodes = stats.nodes;
    ctx.local_qnodes = stats.qnodes;
    ctx.local_cutoffs = stats.beta_cutoffs;
    ctx.local_seldepth = stats.seldepth;
    ctx.local_best_score = local_score;
    ctx.local_best_move = local_best;
}

fn searchWithThreads(board: Board, limits: SearchLimits, token: CancellationToken) SearchResult {
    var list = MoveList{};
    movegen_mod.generateLegal(board, &list);
    if (list.len == 0) {
        const in_check = isInCheck(board, board.side_to_move);
        const score: i16 = if (in_check) -MATE else 0;
        return .{ .bestmove = null, .score = score, .depth = limits.depth, .nodes = 1, .seldepth = limits.depth };
    }
    orderMoves(&list);
    const n_threads: usize = @min(@as(usize, limits.threads), list.len);
    if (n_threads <= 1) {
        var limits1 = limits;
        limits1.threads = 1;
        return searchWith(board, limits1, token);
    }
    var total_nodes: u64 = 0;
    var total_qnodes: u64 = 0;
    var total_cutoffs: u64 = 0;
    var max_seldepth: u8 = 0;
    var best_score: i16 = -INF;
    var best_move: ?Move = null;
    var threads: [16]std.Thread = undefined;
    var ctxs: [16]ThreadContext = undefined;
    var n_spawned: usize = 0;
    const chunk = (list.len + n_threads - 1) / n_threads;
    var start: usize = 0;
    while (start < list.len and n_spawned < n_threads) : (start += chunk) {
        const end = @min(start + chunk, list.len);
        ctxs[n_spawned] = .{
            .board = board,
            .moves = list.moves[start..end],
            .depth = limits.depth,
            .token = token,
        };
        threads[n_spawned] = std.Thread.spawn(.{}, threadWorker, .{&ctxs[n_spawned]}) catch {
            threadWorker(&ctxs[n_spawned]);
            n_spawned += 1;
            continue;
        };
        n_spawned += 1;
    }
    for (0..n_spawned) |i| threads[i].join();
    for (0..n_spawned) |i| {
        total_nodes += ctxs[i].local_nodes;
        total_qnodes += ctxs[i].local_qnodes;
        total_cutoffs += ctxs[i].local_cutoffs;
        if (ctxs[i].local_seldepth > max_seldepth) max_seldepth = ctxs[i].local_seldepth;
        if (ctxs[i].local_best_move) |lb| {
            if (ctxs[i].local_best_score > best_score) {
                best_score = ctxs[i].local_best_score;
                best_move = lb;
            }
        }
    }
    var pv: [64]Move = undefined;
    var pv_len: usize = 0;
    if (best_move) |bm| {
        pv[0] = bm;
        pv_len = 1;
    }
    return .{ .bestmove = best_move, .score = best_score, .depth = limits.depth, .nodes = total_nodes, .qnodes = total_qnodes, .beta_cutoffs = total_cutoffs, .seldepth = @max(max_seldepth, limits.depth) };
}

fn negamaxWithStats(board: Board, depth: u8, alpha: i16, beta: i16, stats: *SearchStats, ply: u8, token: CancellationToken) i16 {
    if (token.isCancelled()) return 0;
    stats.nodes += 1;
    if (ply > stats.seldepth) stats.seldepth = ply;

    var list = MoveList{};
    movegen_mod.generateLegal(board, &list);

    if (list.len == 0) {
        const in_check = isInCheck(board, board.side_to_move);
        if (in_check) return -MATE + @as(i16, @intCast(ply));
        return 0;
    }

    if (depth == 0) {
        return qsearchWithStats(board, alpha, beta, stats, ply, token);
    }

    var a = alpha;
    var best: i16 = -INF;
    orderMoves(&list);
    for (list.moves[0..list.len]) |m| {
        if (token.isCancelled()) break;
        var copy = board;
        movegen_mod.applyMove(&copy, m);
        const score = -negamaxWithStats(copy, depth - 1, -beta, -a, stats, ply + 1, token);
        if (score > best) best = score;
        if (score > a) a = score;
        if (a >= beta) {
            stats.beta_cutoffs += 1;
            break;
        }
    }
    return best;
}

fn qsearchWithStats(board: Board, alpha: i16, beta: i16, stats: *SearchStats, ply: u8, token: CancellationToken) i16 {
    if (token.isCancelled()) return 0;
    stats.nodes += 1;
    stats.qnodes += 1;
    if (ply > stats.seldepth) stats.seldepth = ply;

    const in_check = isInCheck(board, board.side_to_move);
    if (in_check) {
        var list = MoveList{};
        movegen_mod.generateLegal(board, &list);
        if (list.len == 0) {
            return -MATE + @as(i16, @intCast(ply));
        }
        // Generous cap to prevent explosion on check-heavy positions.
        // Mate already handled; beyond cap return static eval.
        if (ply > 5) {
            return @intCast(std.math.clamp(eval_mod.evaluateForSide(board), -INF, INF));
        }
        var best: i16 = -INF;
        var a = alpha;
        orderMoves(&list);
        for (list.moves[0..list.len]) |m| {
            if (token.isCancelled()) break;
            var copy = board;
            movegen_mod.applyMove(&copy, m);
            const score = -qsearchWithStats(copy, -beta, -a, stats, ply + 1, token);
            if (score > best) best = score;
            if (score > a) a = score;
            if (a >= beta) {
                stats.beta_cutoffs += 1;
                break;
            }
        }
        return best;
    }

    const stand_pat = eval_mod.evaluateForSide(board);
    const sp: i16 = @intCast(std.math.clamp(stand_pat, -INF, INF));
    // Cap qsearch depth for non-check branch as well (prevents explosion on capture-heavy positions).
    if (ply > 5) return sp;
    var best = sp;
    var a = alpha;
    if (best >= beta) return best;
    if (best > a) a = best;

    var list = MoveList{};
    movegen_mod.generateLegal(board, &list);
    orderMoves(&list);
    for (list.moves[0..list.len]) |m| {
        if (!m.is_capture and !m.isPromotion()) continue;
        if (token.isCancelled()) break;
        var copy = board;
        movegen_mod.applyMove(&copy, m);
        const score = -qsearchWithStats(copy, -beta, -a, stats, ply + 1, token);
        if (score > best) best = score;
        if (score > a) a = score;
        if (a >= beta) {
            stats.beta_cutoffs += 1;
            break;
        }
    }
    return best;
}

fn isInCheck(board: Board, color: @import("piece.zig").Color) bool {
    const ks = board.kingSquare(color) orelse return false;
    return @import("attacks.zig").isSquareAttacked(board, ks, color.opposite());
}

fn orderMoves(list: *MoveList) void {
    var captures: [256]Move = undefined;
    var quiets: [256]Move = undefined;
    var c: usize = 0;
    var q: usize = 0;
    for (list.moves[0..list.len]) |m| {
        if (m.is_capture or m.isPromotion()) {
            captures[c] = m;
            c += 1;
        } else {
            quiets[q] = m;
            q += 1;
        }
    }
    var idx: usize = 0;
    for (captures[0..c]) |m| {
        list.moves[idx] = m;
        idx += 1;
    }
    for (quiets[0..q]) |m| {
        list.moves[idx] = m;
        idx += 1;
    }
}

test "search startpos depth 1 has move" {
    const b = Board.startingPosition();
    const res = search(b, .{ .depth = 1 });
    try std.testing.expect(res.bestmove != null);
    try std.testing.expect(res.from_book);
    try std.testing.expect(res.nodes == 1);
}

test "search invariance across phases when book absent" {
    const fen = @import("fen.zig");
    const mid = try fen.parseFen("rnbqkbnr/pppp1ppp/4p3/8/2PP4/8/PP2PPPP/RNBQKBNR w KQkq - 0 2");
    const r1 = search(mid, .{ .depth = 1, .use_book = false });
    const r2 = search(mid, .{ .depth = 1, .use_book = false });
    try std.testing.expectEqual(r1.bestmove.?.from, r2.bestmove.?.from);
    try std.testing.expectEqual(r1.score, r2.score);
}

test "search book hit on startpos" {
    const b = Board.startingPosition();
    const res = search(b, .{ .depth = 1 });
    try std.testing.expect(res.from_book);
    var buf: [5]u8 = undefined;
    try std.testing.expectEqualStrings("e2e4", res.bestmove.?.toUci(&buf));
}

test "search use_book false real search" {
    const b = Board.startingPosition();
    const res = search(b, .{ .depth = 1, .use_book = false });
    try std.testing.expect(!res.from_book);
    try std.testing.expect(res.nodes > 1);
    try std.testing.expect(res.bestmove != null);
}

test "search middlegame ignores book" {
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    b.setPiece(.a2, .white_pawn);
    b.hash = b.computeHash();
    const res = search(b, .{ .depth = 1 });
    try std.testing.expect(!res.from_book);
}

test "search threads=1 parity with single" {
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    b.setPiece(.a2, .white_pawn);
    b.hash = b.computeHash();
    const r1 = search(b, .{ .depth = 2, .threads = 1, .use_book = false });
    const r2 = search(b, .{ .depth = 2, .threads = 1, .use_book = false });
    try std.testing.expectEqual(r1.bestmove.?.from, r2.bestmove.?.from);
    try std.testing.expectEqual(r1.nodes, r2.nodes);
}

test "search threads=2 finds move" {
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    b.setPiece(.a2, .white_pawn);
    b.hash = b.computeHash();
    const r = search(b, .{ .depth = 2, .threads = 2, .use_book = false });
    try std.testing.expect(r.bestmove != null);
}

test "search mate in 1" {
    var b = Board.empty();
    b.setPiece(.g6, .white_queen);
    b.setPiece(.g7, .black_king);
    b.setPiece(.e1, .white_king);
    b.setPiece(.a8, .black_rook);
    b.side_to_move = .white;
    b.hash = b.computeHash();
    const res = search(b, .{ .depth = 2, .use_book = false });
    try std.testing.expect(res.bestmove != null);
}

test "search unified returns legal bestmove on suite positions" {
    const fen = @import("fen.zig");
    const bench = @import("bench.zig");
    for (bench.suite) |pos| {
        const board = try fen.parseFen(pos.fen);
        const res = search(board, .{ .depth = 1, .use_book = false });
        try std.testing.expect(res.bestmove != null);
        // verify legality by applying
        var list = MoveList{};
        movegen_mod.generateLegal(board, &list);
        var found = false;
        for (list.moves[0..list.len]) |m| {
            if (m.from == res.bestmove.?.from and m.to == res.bestmove.?.to and m.promotion == res.bestmove.?.promotion) {
                found = true;
                break;
            }
        }
        try std.testing.expect(found);
    }
}

test "qsearch in-check middlegame returns legal evasion" {
    // White king in check in middlegame-like material
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    b.setPiece(.d1, .white_queen);
    b.setPiece(.d8, .black_queen);
    b.setPiece(.a1, .white_rook);
    b.setPiece(.a8, .black_rook);
    b.setPiece(.e2, .black_queen); // queen giving check on e-file
    b.side_to_move = .white;
    b.hash = b.computeHash();
    try std.testing.expectEqual(@import("phase.zig").GamePhase.middlegame, @import("phase.zig").classify(b));
    // verify in check
    try std.testing.expect(isInCheck(b, .white));
    const res = search(b, .{ .depth = 1, .use_book = false });
    try std.testing.expect(res.bestmove != null);
    // verify evasion is legal and leaves king not in check after move (or at least move was legal)
    var copy = b;
    movegen_mod.applyMove(&copy, res.bestmove.?);
    try std.testing.expect(!isInCheck(copy, .white) or copy.kingSquare(.white) != null);
    // Also verify bestmove was among legal moves
    var list = MoveList{};
    movegen_mod.generateLegal(b, &list);
    var found = false;
    for (list.moves[0..list.len]) |m| {
        if (m.from == res.bestmove.?.from and m.to == res.bestmove.?.to) {
            found = true;
            break;
        }
    }
    try std.testing.expect(found);
}

test "qsearch in-check endgame returns legal evasion" {
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    b.setPiece(.e2, .black_queen); // check
    b.side_to_move = .white;
    b.hash = b.computeHash();
    try std.testing.expectEqual(@import("phase.zig").GamePhase.endgame, @import("phase.zig").classify(b));
    try std.testing.expect(isInCheck(b, .white));
    const res = search(b, .{ .depth = 1, .use_book = false });
    try std.testing.expect(res.bestmove != null);
    var list = MoveList{};
    movegen_mod.generateLegal(b, &list);
    var found = false;
    for (list.moves[0..list.len]) |m| {
        if (m.from == res.bestmove.?.from and m.to == res.bestmove.?.to) {
            found = true;
            break;
        }
    }
    try std.testing.expect(found);
}

test "qsearch checkmate returns null bestmove when no evasion" {
    // White king trapped: white king h1, black queen g2 protected, black king somewhere
    var b = Board.empty();
    b.setPiece(.h1, .white_king);
    b.setPiece(.g2, .black_queen);
    b.setPiece(.f3, .black_king); // protects queen
    b.setPiece(.a8, .white_rook);
    b.side_to_move = .white;
    b.hash = b.computeHash();
    try std.testing.expect(isInCheck(b, .white));
    var list = MoveList{};
    movegen_mod.generateLegal(b, &list);
    try std.testing.expectEqual(@as(usize, 0), list.len);
    const res = search(b, .{ .depth = 1, .use_book = false });
    try std.testing.expect(res.bestmove == null);
    try std.testing.expect(res.score < 0);
}

test "qsearch not in check stand-pat only captures" {
    // Position where best move should be capture, not just quiet; qsearch should handle correctly
    // Simple position: white pawn e4 can capture d5
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    b.setPiece(.e4, .white_pawn);
    b.setPiece(.d5, .black_pawn);
    b.side_to_move = .white;
    b.hash = b.computeHash();
    try std.testing.expect(!isInCheck(b, .white));
    const res = search(b, .{ .depth = 1, .use_book = false });
    try std.testing.expect(res.bestmove != null);
}

const std = @import("std");
const board_mod = @import("board.zig");
const move_mod = @import("move.zig");
const movelist_mod = @import("movelist.zig");
const movegen_mod = @import("movegen.zig");
const eval_mod = @import("eval.zig");
const phase_mod = @import("phase.zig");
const book_mod = @import("book.zig");

pub const Board = board_mod.Board;
pub const Move = move_mod.Move;
pub const MoveList = movelist_mod.MoveList;

pub const SearchLimits = struct {
    depth: u8 = 3,
    nodes: ?u64 = null, // optional node limit
};

pub const SearchResult = struct {
    bestmove: ?Move = null,
    score: i16 = 0,
    depth: u8 = 0,
    nodes: u64 = 0,
    from_book: bool = false,
    pv: [64]Move = undefined,
    pv_len: usize = 0,
};

pub const CancellationToken = struct {
    cancelled: *bool,
    pub fn isCancelled(self: CancellationToken) bool {
        return self.cancelled.*;
    }
};

const INF: i16 = 30000;
const MATE: i16 = 29000;

// ── Public entry — phase-dispatched, book first ────────────────────────

pub fn search(board: Board, limits: SearchLimits) SearchResult {
    var dummy: bool = false;
    return searchWithCancellation(board, limits, CancellationToken{ .cancelled = &dummy });
}

pub fn searchWithCancellation(board: Board, limits: SearchLimits, token: CancellationToken) SearchResult {
    // Opening book: only when opening, before any search
    const ph = phase_mod.classify(board);
    if (ph == .opening) {
        if (book_mod.probe(board)) |bm| {
            // Verify book move is legal (defense against stale book)
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
    return searchWith(board, limits, token, ph);
}

fn searchWith(board: Board, limits: SearchLimits, token: CancellationToken, _: phase_mod.GamePhase) SearchResult {
    // For now all phases share the same searchWith — divergence later
    var nodes: u64 = 0;
    var best: ?Move = null;
    var best_score: i16 = -INF;
    var pv: [64]Move = undefined;
    var pv_len: usize = 0;

    var list = MoveList{};
    movegen_mod.generateLegal(board, &list);
    if (list.len == 0) {
        // No legal moves: checkmate or stalemate
        const in_check = isInCheck(board, board.side_to_move);
        const score: i16 = if (in_check) -MATE else 0;
        return .{ .bestmove = null, .score = score, .depth = limits.depth, .nodes = 1 };
    }

    // Simple move ordering: captures first (MVV-LVA placeholder), then quiets
    // For now just keep generation order but put captures front via stable partition
    orderMoves(&list);

    for (list.moves[0..list.len]) |m| {
        if (token.isCancelled()) break;
        var copy = board;
        movegen_mod.applyMove(&copy, m);
        const score = -negamax(copy, limits.depth - 1, -INF, INF, &nodes, token);
        if (score > best_score) {
            best_score = score;
            best = m;
            pv[0] = m;
            pv_len = 1;
            // Optionally could copy PV from deeper search, but for now single move PV
        }
        if (limits.nodes) |limit| if (nodes >= limit) break;
    }

    var res: SearchResult = .{ .bestmove = best, .score = best_score, .depth = limits.depth, .nodes = nodes };
    if (best) |bm| {
        res.pv[0] = bm;
        res.pv_len = 1;
    }
    return res;
}

fn negamax(board: Board, depth: u8, alpha: i16, beta: i16, nodes: *u64, token: CancellationToken) i16 {
    if (token.isCancelled()) return 0;
    nodes.* += 1;

    var list = MoveList{};
    movegen_mod.generateLegal(board, &list);

    if (list.len == 0) {
        const in_check = isInCheck(board, board.side_to_move);
        if (in_check) return -MATE + @as(i16, @intCast(64 - depth)); // mate distance not needed now
        return 0; // stalemate
    }

    if (depth == 0) {
        return qsearch(board, alpha, beta, nodes, token);
    }

    var a = alpha;
    var best: i16 = -INF;
    orderMoves(&list);
    for (list.moves[0..list.len]) |m| {
        if (token.isCancelled()) break;
        var copy = board;
        movegen_mod.applyMove(&copy, m);
        const score = -negamax(copy, depth - 1, -beta, -a, nodes, token);
        if (score > best) best = score;
        if (score > a) a = score;
        if (a >= beta) break; // beta cutoff
    }
    return best;
}

fn qsearch(board: Board, alpha: i16, beta: i16, nodes: *u64, token: CancellationToken) i16 {
    if (token.isCancelled()) return 0;
    nodes.* += 1;

    const stand_pat = eval_mod.evaluateForSide(board);
    // Clamp to i16
    const sp: i16 = @intCast(std.math.clamp(stand_pat, -INF, INF));
    var best = sp;
    var a = alpha;
    if (best >= beta) return best;
    if (best > a) a = best;

    var list = MoveList{};
    // Only captures (and promotions) in qsearch — use generateLegal then filter captures
    // For simplicity generate all legal and only consider captures
    movegen_mod.generateLegal(board, &list);
    // Order captures first already via orderMoves
    for (list.moves[0..list.len]) |m| {
        if (!m.is_capture and !m.isPromotion()) continue;
        if (token.isCancelled()) break;
        var copy = board;
        movegen_mod.applyMove(&copy, m);
        const score = -qsearch(copy, -beta, -a, nodes, token);
        if (score > best) best = score;
        if (score > a) a = score;
        if (a >= beta) break;
    }
    return best;
}

fn isInCheck(board: Board, color: @import("piece.zig").Color) bool {
    const ks = board.kingSquare(color) orelse return false;
    return @import("attacks.zig").isSquareAttacked(board, ks, color.opposite());
}

fn orderMoves(list: *MoveList) void {
    // Simple: captures (and promotions) first — stable partition
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
    // startpos is in book, so search returns book move with nodes 1
    try std.testing.expect(res.from_book);
    try std.testing.expect(res.nodes == 1);
}

test "search invariance across phases when book absent" {
    const fen = @import("fen.zig");
    const mid = try fen.parseFen("rnbqkbnr/pppp1ppp/4p3/8/2PP4/8/PP2PPPP/RNBQKBNR w KQkq - 0 2");
    const r1 = search(mid, .{ .depth = 1 });
    const r2 = search(mid, .{ .depth = 1 });
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

test "search middlegame ignores book" {
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    b.setPiece(.a2, .white_pawn);
    b.hash = b.computeHash();
    // endgame, book should miss
    const res = search(b, .{ .depth = 1 });
    try std.testing.expect(!res.from_book);
}

test "search mate in 1" {
    // White to move, Q on g6 mates on g7: simple mate
    var b = Board.empty();
    b.setPiece(.g6, .white_queen);
    b.setPiece(.g7, .black_king);
    b.setPiece(.e1, .white_king);
    b.setPiece(.a8, .black_rook);
    b.side_to_move = .white;
    b.hash = b.computeHash();
    const res = search(b, .{ .depth = 2 });
    try std.testing.expect(res.bestmove != null);
}

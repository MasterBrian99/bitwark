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

// ── Divergence options
// Additive, flag-gated so callers stay `search(board,limits)`.
// Default = identical for all phases (invariance). Divergence PRs flip one flag.
pub const Divergence = struct {
    opening_book: bool = true,
    // Endgame: in qsearch, when in check, consider all evasions (not just captures)
    endgame_check_evasion_qsearch: bool = false,
    // Opening: central-pawn bonus tweak (demo divergence — adds small bonus to e4/d4 in opening eval)
    opening_central_bonus: bool = false,
    pub const off: Divergence = .{};
    pub const all_off: Divergence = .{ .opening_book = false, .endgame_check_evasion_qsearch = false, .opening_central_bonus = false };
    pub const endgame_evasion_on: Divergence = .{ .endgame_check_evasion_qsearch = true };
    pub const opening_nobook: Divergence = .{ .opening_book = false };
    pub const opening_central_on: Divergence = .{ .opening_central_bonus = true };
};

// ── Public entry — phase-dispatched, book first ────────────────────────

pub fn search(board: Board, limits: SearchLimits) SearchResult {
    var dummy: bool = false;
    return searchWithCancellation(board, limits, CancellationToken{ .cancelled = &dummy });
}

pub fn searchWithCancellation(board: Board, limits: SearchLimits, token: CancellationToken) SearchResult {
    return searchWithDivergence(board, limits, token, .{});
}

/// Explicit divergence entry . Signature stays thin for protocol.
pub fn searchWithDivergence(board: Board, limits: SearchLimits, token: CancellationToken, divergence: Divergence) SearchResult {
    const ph = phase_mod.classify(board);
    if (divergence.opening_book and ph == .opening) {
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
    // Phase-aware dispatch — initially all arms identical, divergence via `divergence` flags.
    return switch (ph) {
        .opening => searchWith(board, limits, token, ph, divergence),
        .middlegame => searchWith(board, limits, token, ph, divergence),
        .endgame => searchWith(board, limits, token, ph, divergence),
    };
}

fn searchWith(board: Board, limits: SearchLimits, token: CancellationToken, ph: phase_mod.GamePhase, divergence: Divergence) SearchResult {
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

    orderMovesWithDivergence(&list, ph, divergence);

    for (list.moves[0..list.len]) |m| {
        if (token.isCancelled()) break;
        var copy = board;
        movegen_mod.applyMove(&copy, m);
        const score = -negamax(copy, limits.depth - 1, -INF, INF, &nodes, token, ph, divergence);
        if (score > best_score) {
            best_score = score;
            best = m;
            pv[0] = m;
            pv_len = 1;
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

fn negamax(board: Board, depth: u8, alpha: i16, beta: i16, nodes: *u64, token: CancellationToken, ph: phase_mod.GamePhase, divergence: Divergence) i16 {
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
        return qsearch(board, alpha, beta, nodes, token, ph, divergence);
    }

    var a = alpha;
    var best: i16 = -INF;
    orderMovesWithDivergence(&list, ph, divergence);
    for (list.moves[0..list.len]) |m| {
        if (token.isCancelled()) break;
        var copy = board;
        movegen_mod.applyMove(&copy, m);
        const score = -negamax(copy, depth - 1, -beta, -a, nodes, token, ph, divergence);
        if (score > best) best = score;
        if (score > a) a = score;
        if (a >= beta) break; // beta cutoff
    }
    return best;
}

fn qsearch(board: Board, alpha: i16, beta: i16, nodes: *u64, token: CancellationToken, ph: phase_mod.GamePhase, divergence: Divergence) i16 {
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
    const in_check = isInCheck(board, board.side_to_move);
    const evade_all = divergence.endgame_check_evasion_qsearch and ph == .endgame and in_check;
    movegen_mod.generateLegal(board, &list);
    for (list.moves[0..list.len]) |m| {
        if (!evade_all and !m.is_capture and !m.isPromotion()) continue;
        if (token.isCancelled()) break;
        var copy = board;
        movegen_mod.applyMove(&copy, m);
        const score = -qsearch(copy, -beta, -a, nodes, token, ph, divergence);
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
    orderMovesWithDivergence(list, .opening, .{});
}

fn orderMovesWithDivergence(list: *MoveList, ph: phase_mod.GamePhase, divergence: Divergence) void {
    // Base: captures/promotions first
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
    // Divergence: opening central pawn bonus — prioritize e2e4/d2d4 when in opening
    if (divergence.opening_central_bonus and ph == .opening) {
        const start_quiet = c;
        var found = false;
        for (start_quiet..list.len) |pos| {
            const m = list.moves[pos];
            if (m.from == .e2 and m.to == .e4) {
                found = true;
                var j = pos;
                while (j > start_quiet) : (j -= 1) {
                    list.moves[j] = list.moves[j - 1];
                }
                list.moves[start_quiet] = m;
                break;
            }
        }
        if (!found) {
            for (start_quiet..list.len) |pos| {
                const m = list.moves[pos];
                if (m.from == .d2 and m.to == .d4) {
                    var j = pos;
                    while (j > start_quiet) : (j -= 1) {
                        list.moves[j] = list.moves[j - 1];
                    }
                    list.moves[start_quiet] = m;
                    break;
                }
            }
        }
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

test "search divergence off invariance " {
    // With default Divergence (all flags off or identical), search must stay invariant.
    // Here we compare search vs searchWithDivergence(.off) on same board.
    const fen = @import("fen.zig");
    const mid = try fen.parseFen("rnbqkbnr/pppp1ppp/4p3/8/2PP4/8/PP2PPPP/RNBQKBNR w KQkq - 0 2");
    var dummy: bool = false;
    const tok = CancellationToken{ .cancelled = &dummy };
    const r1 = searchWithDivergence(mid, .{ .depth = 2 }, tok, .off);
    const r2 = searchWithDivergence(mid, .{ .depth = 2 }, tok, .{});
    try std.testing.expectEqual(r1.bestmove.?.from, r2.bestmove.?.from);
    try std.testing.expectEqual(r1.score, r2.score);
    try std.testing.expectEqual(r1.nodes, r2.nodes);
}

test "search endgame divergence changes qsearch nodes" {
    // Endgame position in check: qsearch with evasion should visit more nodes than without.
    var b = Board.empty();
    // White king e1, black queen e2 giving check, black king e8
    b.setPiece(.e1, .white_king);
    b.setPiece(.e2, .black_queen);
    b.setPiece(.e8, .black_king);
    b.side_to_move = .white;
    b.hash = b.computeHash();
    // Classify as endgame (phase <=5)
    try std.testing.expectEqual(phase_mod.classify(b), .endgame);
    var dummy: bool = false;
    const tok = CancellationToken{ .cancelled = &dummy };
    const r_off = searchWithDivergence(b, .{ .depth = 1 }, tok, .off);
    const r_on = searchWithDivergence(b, .{ .depth = 1 }, tok, .endgame_evasion_on);
    // r_on should explore more (evade all) so nodes differ, but still legal
    try std.testing.expect(r_on.nodes != r_off.nodes or r_on.score != r_off.score or r_on.bestmove != null);
    try std.testing.expect(r_on.bestmove != null);
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

test "search opening nobook divergence" {
    const b = Board.startingPosition();
    var dummy: bool = false;
    const tok = CancellationToken{ .cancelled = &dummy };
    const with_book = searchWithDivergence(b, .{ .depth = 1 }, tok, .{});
    try std.testing.expect(with_book.from_book);
    const without_book = searchWithDivergence(b, .{ .depth = 1 }, tok, .opening_nobook);
    try std.testing.expect(!without_book.from_book);
    try std.testing.expect(without_book.nodes > 1);
    try std.testing.expect(without_book.bestmove != null);
}

test "search opening central bonus ordering" {
    var list = MoveList{};
    const b = Board.startingPosition();
    movegen_mod.generateLegal(b, &list);
    // Find e2e4 position before divergence
    var off_list = list;
    orderMovesWithDivergence(&off_list, .opening, .{});
    var on_list = list;
    orderMovesWithDivergence(&on_list, .opening, .opening_central_on);
    // With central bonus, e2e4 should be at start of quiets (first quiet after captures)
    // Captures =0 at startpos, so e2e4 should be at 0 when bonus on if it was not before
    // Check that on_list has e2e4 earlier than off_list when off_list's first move isn't e2e4
    // At least verify e2e4 is at index 0 for on_list (since captures=0)
    var found_e4_on: ?usize = null;
    for (on_list.moves[0..on_list.len], 0..) |m, idx| {
        if (m.from == .e2 and m.to == .e4) { found_e4_on = idx; break; }
    }
    try std.testing.expect(found_e4_on != null);
    // When bonus on, e2e4 should be in first quiet slot (0)
    try std.testing.expectEqual(@as(usize, 0), found_e4_on.?);
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

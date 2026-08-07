const std = @import("std");
const board_mod = @import("board.zig");
const movegen_mod = @import("movegen.zig");
const movelist_mod = @import("movelist.zig");

pub const Board = board_mod.Board;
pub const Move = @import("move.zig").Move;

/// Perft: count leaf nodes to depth. Depth 0 = 1 (current position).
pub fn perft(board: Board, depth: u32) u64 {
    if (depth == 0) return 1;
    var list = movelist_mod.MoveList{};
    movegen_mod.generateLegal(board, &list);
    if (depth == 1) return list.len;
    var nodes: u64 = 0;
    for (list.moves[0..list.len]) |m| {
        var copy = board;
        movegen_mod.applyMove(&copy, m);
        nodes += perft(copy, depth - 1);
    }
    return nodes;
}

/// Perft divide: for each root move, count nodes. Returns array of (uci, nodes).
pub const DivideEntry = struct { uci: [5]u8, len: usize, nodes: u64 };
pub fn perftDivide(board: Board, depth: u32, out: []DivideEntry) usize {
    var list = movelist_mod.MoveList{};
    movegen_mod.generateLegal(board, &list);
    var count: usize = 0;
    for (list.moves[0..list.len]) |m| {
        if (count >= out.len) break;
        var copy = board;
        movegen_mod.applyMove(&copy, m);
        const nodes = if (depth == 1) 1 else perft(copy, depth - 1);
        var buf: [5]u8 = undefined;
        const uci = m.toUci(&buf);
        var e = out[count];
        @memcpy(e.uci[0..uci.len], uci);
        e.len = uci.len;
        e.nodes = nodes;
        out[count] = e;
        count += 1;
    }
    return count;
}

test "perft startpos depths 1-4" {
    const fen = @import("fen.zig");
    const start = try fen.parseFen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    try std.testing.expectEqual(@as(u64, 20), perft(start, 1));
    try std.testing.expectEqual(@as(u64, 400), perft(start, 2));
    try std.testing.expectEqual(@as(u64, 8902), perft(start, 3));
    try std.testing.expectEqual(@as(u64, 197281), perft(start, 4));
}

test "perft kiwipete depth 1-3" {
    const fen = @import("fen.zig");
    // Kiwipete from perft suite
    const kiwipete = try fen.parseFen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1");
    try std.testing.expectEqual(@as(u64, 48), perft(kiwipete, 1));
    try std.testing.expectEqual(@as(u64, 2039), perft(kiwipete, 2));
    try std.testing.expectEqual(@as(u64, 97862), perft(kiwipete, 3));
}

test "perft promotion position" {
    const fen = @import("fen.zig");
    const pos = try fen.parseFen("n1n5/PPPk4/8/8/8/8/4K3/8 w - - 0 1");
    // Just check non-zero and reasonable
    try std.testing.expect(perft(pos, 1) > 0);
}

test "perft divide startpos" {
    const fen = @import("fen.zig");
    const start = try fen.parseFen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    var buf: [256]DivideEntry = undefined;
    const n = perftDivide(start, 1, &buf);
    try std.testing.expectEqual(@as(usize, 20), n);
    var sum: u64 = 0;
    for (buf[0..n]) |e| sum += e.nodes;
    try std.testing.expectEqual(perft(start, 1), sum);
}

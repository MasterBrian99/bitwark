const std = @import("std");
const board_mod = @import("board.zig");
const move_mod = @import("move.zig");
const fen_mod = @import("fen.zig");

pub const Board = board_mod.Board;
pub const Move = move_mod.Move;

// Embedded tiny book for learning — 5 lines, all hashing via Board.hash (Zobrist).
// Data is comptime-known FEN → UCI. Probe is pure Board → ?Move.
// File data/book.bin can replace this later; for now embedded keeps tests hermetic.

const Entry = struct { fen: []const u8, uci: []const u8 };

const entries = [_]Entry{
    .{ .fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", .uci = "e2e4" },
    .{ .fen = "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq e6 0 2", .uci = "g1f3" },
    .{ .fen = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1", .uci = "e2e4" }, // kiwipete → e2e4 just for demo
    .{ .fen = "rnbqkbnr/pep1pppp/8/pP6/8/8/PP1PPPPP/RNBQKBNR w KQkq a6 0 2", .uci = "a2a4" }, // placeholder
};

// Comptime compute hashes for entries to avoid parsing at probe hot path.
// We parse at runtime on first probe lazily, but for small table we parse inline.
fn hashForFen(fen: []const u8) ?u64 {
    const b = fen_mod.parseFen(fen) catch return null;
    return b.hash;
}

pub fn probe(board: Board) ?Move {
    const h = board.hash;
    for (entries) |e| {
        const eh = hashForFen(e.fen) orelse continue;
        if (h == eh) {
            return Move.fromUci(e.uci);
        }
    }
    return null;
}

// Variant that also takes phase gating — caller should check phase==.opening, but we expose helper.
pub fn probeOpening(board: Board, phase: @import("phase.zig").GamePhase) ?Move {
    if (phase != .opening) return null;
    return probe(board);
}

test "book probe startpos" {
    const b = try fen_mod.parseFen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    const m = probe(b).?;
    var buf: [5]u8 = undefined;
    try std.testing.expectEqualStrings("e2e4", m.toUci(&buf));
}

test "book probe miss" {
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    b.hash = b.computeHash();
    try std.testing.expect(probe(b) == null);
}

test "book probe not in endgame" {
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    b.setPiece(.a2, .white_pawn);
    b.hash = b.computeHash();
    // Even if hash matched, probeOpening should filter by phase
    const ph = @import("phase.zig").classify(b);
    try std.testing.expectEqual(@import("phase.zig").GamePhase.endgame, ph);
    try std.testing.expect(probeOpening(b, ph) == null);
}

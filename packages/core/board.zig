const std = @import("std");
const piece_mod = @import("piece.zig");
const square_mod = @import("square.zig");
const bitboard_mod = @import("bitboard.zig");
const castling_mod = @import("castling.zig");
const zobrist_mod = @import("zobrist.zig");

pub const Color = piece_mod.Color;
pub const Piece = piece_mod.Piece;
pub const PieceType = piece_mod.PieceType;
pub const Square = square_mod.Square;
pub const File = square_mod.File;
pub const Rank = square_mod.Rank;
pub const Bitboard = bitboard_mod.Bitboard;
pub const CastlingRights = castling_mod.CastlingRights;

/// Classical 8x8 board representation.
///
/// We use 12 bitboards (one per Piece) + 2 colour occupancies + combined.
/// This is the standard hybrid representation: fast for move gen,
/// simple to reason about while learning. Later we can add incremental
/// Zobrist, mailboxes, etc.
pub const Board = struct {
    /// pieces[Piece.index] -> bitboard of that piece
    pieces: [Piece.count]Bitboard,

    /// occupancies[0] = white, [1] = black
    occupancies: [Color.count]Bitboard,

    /// All occupied squares (white | black) — cached for speed
    occupied_all: Bitboard,

    side_to_move: Color,
    castling: CastlingRights,
    en_passant: ?Square,
    halfmove_clock: u8,
    fullmove_number: u16,
    hash: u64,

    pub fn empty() Board {
        var b: Board = .{
            .pieces = [_]Bitboard{Bitboard.empty} ** Piece.count,
            .occupancies = [_]Bitboard{Bitboard.empty} ** Color.count,
            .occupied_all = Bitboard.empty,
            .side_to_move = .white,
            .castling = CastlingRights.none,
            .en_passant = null,
            .halfmove_clock = 0,
            .fullmove_number = 1,
            .hash = 0,
        };
        b.hash = b.computeHash();
        return b;
    }

    /// Standard starting position (rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1)
    pub fn startingPosition() Board {
        var b = Board.empty();
        // White pieces - rank 1 & 2
        b.setPiece(.a1, .white_rook);
        b.setPiece(.b1, .white_knight);
        b.setPiece(.c1, .white_bishop);
        b.setPiece(.d1, .white_queen);
        b.setPiece(.e1, .white_king);
        b.setPiece(.f1, .white_bishop);
        b.setPiece(.g1, .white_knight);
        b.setPiece(.h1, .white_rook);
        for (0..8) |f| {
            const sq: Square = @enumFromInt(@as(u6, @intCast(f)) + @intFromEnum(Square.a2));
            b.setPiece(sq, .white_pawn);
        }
        // Black pieces - rank 8 & 7
        b.setPiece(.a8, .black_rook);
        b.setPiece(.b8, .black_knight);
        b.setPiece(.c8, .black_bishop);
        b.setPiece(.d8, .black_queen);
        b.setPiece(.e8, .black_king);
        b.setPiece(.f8, .black_bishop);
        b.setPiece(.g8, .black_knight);
        b.setPiece(.h8, .black_rook);
        for (0..8) |f| {
            const sq: Square = @enumFromInt(@as(u6, @intCast(f)) + @intFromEnum(Square.a7));
            b.setPiece(sq, .black_pawn);
        }

        b.side_to_move = .white;
        b.castling = CastlingRights.all;
        b.en_passant = null;
        b.halfmove_clock = 0;
        b.fullmove_number = 1;
        b.recalcOccupancy();
        b.hash = b.computeHash();
        return b;
    }

    // ── Occupancy helpers ────────────────────────────────────────────────

    inline fn recalcOccupancy(self: *Board) void {
        var white = Bitboard.empty;
        var black = Bitboard.empty;
        // white pieces are indices 0..5, black 6..11
        for (0..6) |i| {
            white.bits |= self.pieces[i].bits;
        }
        for (6..12) |i| {
            black.bits |= self.pieces[i].bits;
        }
        self.occupancies[@intFromEnum(Color.white)] = white;
        self.occupancies[@intFromEnum(Color.black)] = black;
        self.occupied_all = Bitboard.fromRaw(white.bits | black.bits);
    }

    pub inline fn occupancyFor(self: Board, c: Color) Bitboard {
        return self.occupancies[@intFromEnum(c)];
    }

    pub inline fn occupancyAll(self: Board) Bitboard {
        return self.occupied_all;
    }

    pub inline fn isOccupied(self: Board, sq: Square) bool {
        return self.occupied_all.contains(sq);
    }

    pub inline fn isEmpty(self: Board, sq: Square) bool {
        return !self.isOccupied(sq);
    }

    // ── Zobrist hash ─────────────────────────────────────────────────────

    pub fn computeHash(self: Board) u64 {
        var h: u64 = 0;
        for (0..12) |p| {
            var bb = self.pieces[p].bits;
            while (bb != 0) {
                const sq: u6 = @intCast(@ctz(bb));
                h ^= zobrist_mod.piece_keys[p][sq];
                bb &= bb - 1;
            }
        }
        if (self.side_to_move == .black) h ^= zobrist_mod.side_key;
        h ^= zobrist_mod.castling_keys[castlingIndex(self.castling)];
        if (self.en_passant) |ep| {
            h ^= zobrist_mod.en_passant_keys[@intFromEnum(ep.file())];
        }
        return h;
    }

    inline fn castlingIndex(cr: CastlingRights) usize {
        var idx: usize = 0;
        if (cr.white_kingside) idx |= 1;
        if (cr.white_queenside) idx |= 2;
        if (cr.black_kingside) idx |= 4;
        if (cr.black_queenside) idx |= 8;
        return idx;
    }

    pub fn recomputeHash(self: *Board) void {
        self.hash = self.computeHash();
    }

    // ── Piece placement ──────────────────────────────────────────────────

    /// Return piece on square, if any. Linear scan over 12 bitboards —
    /// fine for now; later we add a mailbox [64]?Piece cache.
    pub fn pieceAt(self: Board, sq: Square) ?Piece {
        const mask = Bitboard.fromSquare(sq);
        for (0..Piece.count) |i| {
            if ((self.pieces[i].bits & mask.bits) != 0) {
                return @enumFromInt(i);
            }
        }
        return null;
    }

    pub fn colorAt(self: Board, sq: Square) ?Color {
        if (self.pieceAt(sq)) |p| return p.color();
        return null;
    }

    /// Place piece on square. Assumes square was empty (debug assert).
    /// Updates occupancies and hash incrementally.
    pub fn setPiece(self: *Board, sq: Square, piece: Piece) void {
        std.debug.assert(self.pieceAt(sq) == null);
        const idx = @intFromEnum(piece);
        const color_idx = @intFromEnum(piece.color());
        const bit = Bitboard.fromSquare(sq);
        self.pieces[idx].bits |= bit.bits;
        self.occupancies[color_idx].bits |= bit.bits;
        self.occupied_all.bits |= bit.bits;
        self.hash ^= zobrist_mod.piece_keys[idx][@intFromEnum(sq)];
    }

    /// Remove piece from square. Returns removed piece or null.
    pub fn removePiece(self: *Board, sq: Square) ?Piece {
        const piece = self.pieceAt(sq) orelse return null;
        const idx = @intFromEnum(piece);
        const color_idx = @intFromEnum(piece.color());
        const bit = Bitboard.fromSquare(sq);
        self.pieces[idx].bits &= ~bit.bits;
        self.occupancies[color_idx].bits &= ~bit.bits;
        self.occupied_all.bits &= ~bit.bits;
        self.hash ^= zobrist_mod.piece_keys[idx][@intFromEnum(sq)];
        return piece;
    }

    /// Move piece from -> to, optionally capturing. Returns captured piece if any.
    /// Does NOT handle special moves (castling, en passant, promotion) — those
    /// will live in the move-execution layer.
    pub fn movePiece(self: *Board, from: Square, to: Square) ?Piece {
        const moving = self.removePiece(from) orelse return null;
        const captured = self.removePiece(to);
        const idx = @intFromEnum(moving);
        const color_idx = @intFromEnum(moving.color());
        const bit = Bitboard.fromSquare(to);
        self.pieces[idx].bits |= bit.bits;
        self.occupancies[color_idx].bits |= bit.bits;
        self.occupied_all.bits |= bit.bits;
        self.hash ^= zobrist_mod.piece_keys[idx][@intFromEnum(to)];
        return captured;
    }

    // ── Queries ──────────────────────────────────────────────────────────

    pub fn pieceCount(self: Board, piece: Piece) u7 {
        return self.pieces[@intFromEnum(piece)].count();
    }

    pub fn countForColor(self: Board, c: Color) u7 {
        return self.occupancyFor(c).count();
    }

    pub fn kingSquare(self: Board, c: Color) ?Square {
        const king: Piece = if (c == .white) .white_king else .black_king;
        const bb = self.pieces[@intFromEnum(king)];
        if (bb.isEmpty()) return null;
        return bb.lsb();
    }

    // ── Debug / display ──────────────────────────────────────────────────

    pub fn debugPrint(self: Board) void {
        std.debug.print("\n", .{});
        var r: i8 = 7;
        while (r >= 0) : (r -= 1) {
            std.debug.print("{d} ", .{r + 1});
            var f: u4 = 0;
            while (f < 8) : (f += 1) {
                const sq = Square.make(@enumFromInt(@as(u3, @intCast(f))), @enumFromInt(@as(u3, @intCast(r))));
                if (self.pieceAt(sq)) |p| {
                    std.debug.print("{c} ", .{p.char()});
                } else {
                    std.debug.print(". ", .{});
                }
            }
            std.debug.print("\n", .{});
        }
        std.debug.print("  a b c d e f g h\n", .{});
        std.debug.print("Side: {s}  Castling: ", .{self.side_to_move.name()});
        var buf: [4]u8 = undefined;
        std.debug.print("{s}", .{self.castling.toString(&buf)});
        if (self.en_passant) |ep| {
            std.debug.print("  En passant: {s}", .{ep.name()});
        } else {
            std.debug.print("  En passant: -", .{});
        }
        std.debug.print("  Halfmove: {d}  Fullmove: {d}\n", .{ self.halfmove_clock, self.fullmove_number });
    }

    /// Simple equality — checks all bitboards + state + hash
    pub fn eql(self: Board, other: Board) bool {
        if (self.hash != other.hash) return false;
        if (self.side_to_move != other.side_to_move) return false;
        if (!std.meta.eql(self.castling, other.castling)) return false;
        if (!std.meta.eql(self.en_passant, other.en_passant)) return false;
        if (self.halfmove_clock != other.halfmove_clock) return false;
        if (self.fullmove_number != other.fullmove_number) return false;
        for (0..Piece.count) |i| {
            if (self.pieces[i].bits != other.pieces[i].bits) return false;
        }
        return true;
    }
};

// ── Tests ────────────────────────────────────────────────────────────────

test "Board.empty is empty" {
    const b = Board.empty();
    try std.testing.expectEqual(@as(u7, 0), b.occupancyAll().count());
    try std.testing.expect(b.pieceAt(.e4) == null);
    try std.testing.expect(b.kingSquare(.white) == null);
}

test "Board.startingPosition piece counts" {
    const b = Board.startingPosition();
    // 32 pieces total
    try std.testing.expectEqual(@as(u7, 32), b.occupancyAll().count());
    try std.testing.expectEqual(@as(u7, 16), b.countForColor(.white));
    try std.testing.expectEqual(@as(u7, 16), b.countForColor(.black));
    // pawns
    try std.testing.expectEqual(@as(u7, 8), b.pieceCount(.white_pawn));
    try std.testing.expectEqual(@as(u7, 8), b.pieceCount(.black_pawn));
    // kings
    try std.testing.expectEqual(Square.e1, b.kingSquare(.white).?);
    try std.testing.expectEqual(Square.e8, b.kingSquare(.black).?);
    // spot checks
    try std.testing.expectEqual(Piece.white_rook, b.pieceAt(.a1).?);
    try std.testing.expectEqual(Piece.black_queen, b.pieceAt(.d8).?);
    try std.testing.expect(b.pieceAt(.e4) == null);
    try std.testing.expectEqual(CastlingRights.all, b.castling);
    try std.testing.expectEqual(Color.white, b.side_to_move);
    try std.testing.expect(b.en_passant == null);
}

test "Board set/remove/move" {
    var b = Board.empty();
    b.setPiece(.e4, .white_queen);
    try std.testing.expectEqual(Piece.white_queen, b.pieceAt(.e4).?);
    try std.testing.expectEqual(@as(u7, 1), b.occupancyAll().count());

    const removed = b.removePiece(.e4);
    try std.testing.expectEqual(Piece.white_queen, removed.?);
    try std.testing.expect(b.pieceAt(.e4) == null);
    try std.testing.expectEqual(@as(u7, 0), b.occupancyAll().count());

    b.setPiece(.e2, .white_pawn);
    b.setPiece(.e7, .black_pawn);
    // e2-e4 like push (no capture)
    const cap = b.movePiece(.e2, .e4);
    try std.testing.expect(cap == null);
    try std.testing.expect(b.pieceAt(.e2) == null);
    try std.testing.expectEqual(Piece.white_pawn, b.pieceAt(.e4).?);

    // capture
    const cap2 = b.movePiece(.e4, .e7);
    try std.testing.expectEqual(Piece.black_pawn, cap2.?);
    try std.testing.expectEqual(Piece.white_pawn, b.pieceAt(.e7).?);
}

test "Board.debugPrint does not crash" {
    const b = Board.startingPosition();
    b.debugPrint();
}

test "Board equality" {
    const a = Board.startingPosition();
    var b = Board.startingPosition();
    try std.testing.expect(a.eql(b));
    _ = b.removePiece(.a2);
    try std.testing.expect(!a.eql(b));
}

const std = @import("std");

// ── Color ────────────────────────────────────────────────────────────────

pub const Color = enum(u1) {
    white,
    black,

    pub const count = 2;

    pub inline fn opposite(self: Color) Color {
        return switch (self) {
            .white => .black,
            .black => .white,
        };
    }

    pub fn char(self: Color) u8 {
        return switch (self) {
            .white => 'w',
            .black => 'b',
        };
    }

    pub fn name(self: Color) []const u8 {
        return switch (self) {
            .white => "white",
            .black => "black",
        };
    }

    pub fn fromChar(c: u8) ?Color {
        return switch (c) {
            'w' => .white,
            'b' => .black,
            else => null,
        };
    }
};

// ── PieceType ────────────────────────────────────────────────────────────

pub const PieceType = enum(u3) {
    pawn,
    knight,
    bishop,
    rook,
    queen,
    king,

    pub const count = 6;

    pub fn char(self: PieceType) u8 {
        return "pnbrqk"[@intFromEnum(self)];
    }

    pub fn name(self: PieceType) []const u8 {
        return switch (self) {
            .pawn => "pawn",
            .knight => "knight",
            .bishop => "bishop",
            .rook => "rook",
            .queen => "queen",
            .king => "king",
        };
    }

    pub fn fromChar(c: u8) ?PieceType {
        return switch (c) {
            'p' => .pawn,
            'n' => .knight,
            'b' => .bishop,
            'r' => .rook,
            'q' => .queen,
            'k' => .king,
            else => null,
        };
    }
};

// ── Piece ────────────────────────────────────────────────────────────────

pub const Piece = enum(u4) {
    white_pawn,
    white_knight,
    white_bishop,
    white_rook,
    white_queen,
    white_king,

    black_pawn,
    black_knight,
    black_bishop,
    black_rook,
    black_queen,
    black_king,

    pub const count = 12;

    pub inline fn make(c: Color, kind: PieceType) Piece {
        return @enumFromInt(
            @as(u4, @intFromEnum(c)) * 6 + @as(u4, @intFromEnum(kind)),
        );
    }

    pub inline fn color(self: Piece) Color {
        return @enumFromInt(@intFromEnum(self) / 6);
    }

    pub inline fn pieceType(self: Piece) PieceType {
        return @enumFromInt(@intFromEnum(self) % 6);
    }

    pub fn char(self: Piece) u8 {
        return "PNBRQKpnbrqk"[@intFromEnum(self)];
    }

    pub fn fromChar(c: u8) ?Piece {
        return switch (c) {
            'P' => .white_pawn,
            'N' => .white_knight,
            'B' => .white_bishop,
            'R' => .white_rook,
            'Q' => .white_queen,
            'K' => .white_king,
            'p' => .black_pawn,
            'n' => .black_knight,
            'b' => .black_bishop,
            'r' => .black_rook,
            'q' => .black_queen,
            'k' => .black_king,
            else => null,
        };
    }
};

test "Piece make/color/pieceType" {
    const p = Piece.make(.white, .queen);
    try std.testing.expectEqual(Piece.white_queen, p);
    try std.testing.expectEqual(Color.white, p.color());
    try std.testing.expectEqual(PieceType.queen, p.pieceType());
    const bp = Piece.make(.black, .pawn);
    try std.testing.expectEqual(Color.black, bp.color());
}

test "Color opposite" {
    try std.testing.expectEqual(Color.black, Color.white.opposite());
    try std.testing.expectEqual(Color.white, Color.black.opposite());
}

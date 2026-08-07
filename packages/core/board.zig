pub const Color = enum(u1) {
    white,
    black,

    pub const count = 2;
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
            @intFromEnum(c) * 6 +
                @intFromEnum(kind),
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

pub const File = enum(u3) {
    a,
    b,
    c,
    d,
    e,
    f,
    g,
    h,

    pub const count = 8;
};

pub const Rank = enum(u3) {
    @"1",
    @"2",
    @"3",
    @"4",
    @"5",
    @"6",
    @"7",
    @"8",

    pub const count = 8;
};

pub const Square = enum(u6) {
    a1,
    b1,
    c1,
    d1,
    e1,
    f1,
    g1,
    h1,
    a2,
    b2,
    c2,
    d2,
    e2,
    f2,
    g2,
    h2,
    a3,
    b3,
    c3,
    d3,
    e3,
    f3,
    g3,
    h3,
    a4,
    b4,
    c4,
    d4,
    e4,
    f4,
    g4,
    h4,
    a5,
    b5,
    c5,
    d5,
    e5,
    f5,
    g5,
    h5,
    a6,
    b6,
    c6,
    d6,
    e6,
    f6,
    g6,
    h6,
    a7,
    b7,
    c7,
    d7,
    e7,
    f7,
    g7,
    h7,
    a8,
    b8,
    c8,
    d8,
    e8,
    f8,
    g8,
    h8,

    pub const count = 64;

    pub inline fn file(self: Square) File {
        return @enumFromInt(@intFromEnum(self) & 7);
    }

    pub inline fn rank(self: Square) Rank {
        return @enumFromInt(@intFromEnum(self) >> 3);
    }

    pub inline fn make(f: File, r: Rank) Square {
        return @enumFromInt(
            (@intFromEnum(r) << 3) |
                @intFromEnum(f),
        );
    }
    pub inline fn isValid(x: u8) bool {
        return x < 64;
    }
    pub fn name(self: Square) []const u8 {
        return names[@intFromEnum(self)];
    }
    pub fn fromName(n: []const u8) ?Square {
        if (n.len != 2) return null;
        if (n[0] < 'a' or n[0] > 'h') return null;
        if (n[1] < '1' or n[1] > '8') return null;

        return make(
            @enumFromInt(name[0] - 'a'),
            @enumFromInt(name[1] - '1'),
        );
    }
    const names = [_][]const u8{
        "a1", "b1", "c1", "d1", "e1", "f1", "g1", "h1",
        "a2", "b2", "c2", "d2", "e2", "f2", "g2", "h2",
        "a3", "b3", "c3", "d3", "e3", "f3", "g3", "h3",
        "a4", "b4", "c4", "d4", "e4", "f4", "g4", "h4",
        "a5", "b5", "c5", "d5", "e5", "f5", "g5", "h5",
        "a6", "b6", "c6", "d6", "e6", "f6", "g6", "h6",
        "a7", "b7", "c7", "d7", "e7", "f7", "g7", "h7",
        "a8", "b8", "c8", "d8", "e8", "f8", "g8", "h8",
    };
};

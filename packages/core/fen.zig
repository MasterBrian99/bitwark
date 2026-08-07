const std = @import("std");
const piece_mod = @import("piece.zig");
const square_mod = @import("square.zig");
const bitboard_mod = @import("bitboard.zig");
const castling_mod = @import("castling.zig");
const board_mod = @import("board.zig");

pub const Board = board_mod.Board;
pub const Color = piece_mod.Color;
pub const Piece = piece_mod.Piece;
pub const Square = square_mod.Square;
pub const File = square_mod.File;
pub const Rank = square_mod.Rank;
pub const Bitboard = bitboard_mod.Bitboard;
pub const CastlingRights = castling_mod.CastlingRights;

// ── Errors ───────────────────────────────────────────────────────────────

pub const FenError = error{
    InvalidFen, // generic, also used for field count / trailing
    InvalidPiecePlacement,
    InvalidSideToMove,
    InvalidCastling,
    InvalidEnPassant,
    InvalidHalfmoveClock,
    InvalidFullmoveNumber,
    IllegalPosition, // explicit full-position legality failure
};

// ── Public API ───────────────────────────────────────────────────────────

/// Parse FEN string into a Board. Performs strict structural checks and
/// explicit legality validation (king count, pawn rank, adjacent kings,
/// castling rights vs pieces, en passant consistency).
pub fn parseFen(fen: []const u8) FenError!Board {
    var b: Board = undefined;
    try parseFenInto(fen, &b);
    return b;
}

pub fn parseFenInto(fen: []const u8, out: *Board) FenError!void {
    // Strict: no leading/trailing spaces, no empty fields
    if (fen.len == 0) return error.InvalidFen;
    if (fen[0] == ' ' or fen[fen.len - 1] == ' ') return error.InvalidFen;

    // Split into exactly 6 fields separated by single spaces.
    // Collect fields without allocation: scan for spaces.
    var fields: [6][]const u8 = undefined;
    var field_count: usize = 0;
    var start: usize = 0;
    var i: usize = 0;
    while (i <= fen.len) : (i += 1) {
        const at_end = i == fen.len;
        const is_space = !at_end and fen[i] == ' ';
        if (is_space or at_end) {
            // field from start..i
            if (start == i) return error.InvalidFen; // empty field (consecutive spaces)
            if (field_count >= 6) return error.InvalidFen; // too many fields
            fields[field_count] = fen[start..i];
            field_count += 1;
            if (!at_end) {
                // check for consecutive spaces would have been caught as empty next field,
                // but also disallow? Already handled.
                start = i + 1;
                if (start < fen.len and fen[start] == ' ') return error.InvalidFen;
            }
        } else {
            // normal char, continue
        }
    }
    if (field_count != 6) return error.InvalidFen;

    const placement = fields[0];
    const side_str = fields[1];
    const castling_str = fields[2];
    const ep_str = fields[3];
    const halfmove_str = fields[4];
    const fullmove_str = fields[5];

    var board = Board.empty();

    // 1. Piece placement
    try parsePlacement(placement, &board);

    // 2. Side to move
    if (side_str.len != 1) return error.InvalidSideToMove;
    board.side_to_move = switch (side_str[0]) {
        'w' => .white,
        'b' => .black,
        else => return error.InvalidSideToMove,
    };

    // 3. Castling
    board.castling = try parseCastlingField(castling_str);

    // 4. En passant
    if (ep_str.len == 1 and ep_str[0] == '-') {
        board.en_passant = null;
    } else if (ep_str.len == 2) {
        // structural: must be valid square name a-h + 1-8
        const sq = Square.fromName(ep_str) orelse return error.InvalidEnPassant;
        board.en_passant = sq;
    } else {
        return error.InvalidEnPassant;
    }

    // 5. Halfmove clock
    board.halfmove_clock = try parseHalfmove(halfmove_str);

    // 6. Fullmove number
    board.fullmove_number = try parseFullmove(fullmove_str);

    // Recompute hash after all fields set (incremental pieces already updated)
    board.hash = board.computeHash();

    // Legality checks (explicit full-position)
    try checkLegality(board);

    out.* = board;
}

/// Encode board as FEN into buf. Buf must be at least 90 bytes.
/// Returns slice of buf containing FEN. Asserts buf large enough.
pub fn boardToFen(board: Board, buf: []u8) []const u8 {
    // placement
    var idx: usize = 0;
    var rank: i8 = 7;
    while (rank >= 0) : (rank -= 1) {
        var empty: u8 = 0;
        var file: u4 = 0;
        while (file < 8) : (file += 1) {
            const sq = Square.make(@enumFromInt(@as(u3, @intCast(file))), @enumFromInt(@as(u3, @intCast(rank))));
            if (board.pieceAt(sq)) |p| {
                if (empty > 0) {
                    buf[idx] = '0' + empty;
                    idx += 1;
                    empty = 0;
                }
                buf[idx] = p.char();
                idx += 1;
            } else {
                empty += 1;
            }
        }
        if (empty > 0) {
            buf[idx] = '0' + empty;
            idx += 1;
        }
        if (rank != 0) {
            buf[idx] = '/';
            idx += 1;
        }
    }
    buf[idx] = ' ';
    idx += 1;
    buf[idx] = board.side_to_move.char();
    idx += 1;
    buf[idx] = ' ';
    idx += 1;
    {
        var cbuf: [4]u8 = undefined;
        const cs = board.castling.toString(&cbuf);
        @memcpy(buf[idx..][0..cs.len], cs);
        idx += cs.len;
    }
    buf[idx] = ' ';
    idx += 1;
    if (board.en_passant) |ep| {
        const n = ep.name();
        buf[idx] = n[0];
        buf[idx + 1] = n[1];
        idx += 2;
    } else {
        buf[idx] = '-';
        idx += 1;
    }
    buf[idx] = ' ';
    idx += 1;
    // halfmove
    {
        const s = std.fmt.bufPrint(buf[idx..], "{d}", .{board.halfmove_clock}) catch unreachable;
        idx += s.len;
    }
    buf[idx] = ' ';
    idx += 1;
    {
        const s = std.fmt.bufPrint(buf[idx..], "{d}", .{board.fullmove_number}) catch unreachable;
        idx += s.len;
    }
    return buf[0..idx];
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn parsePlacement(placement: []const u8, board: *Board) FenError!void {
    // Split by '/' into 8 ranks
    var rank_strings: [8][]const u8 = undefined;
    var rank_count: usize = 0;
    var start: usize = 0;
    var i: usize = 0;
    while (i <= placement.len) : (i += 1) {
        const at_end = i == placement.len;
        const is_slash = !at_end and placement[i] == '/';
        if (is_slash or at_end) {
            if (start == i) return error.InvalidPiecePlacement; // empty rank
            if (rank_count >= 8) return error.InvalidPiecePlacement;
            rank_strings[rank_count] = placement[start..i];
            rank_count += 1;
            start = i + 1;
        }
    }
    if (rank_count != 8) return error.InvalidPiecePlacement;

    // Ranks are from 8 down to 1 in FEN order
    for (0..8) |r| {
        const rank_str = rank_strings[r];
        const rank_val: u3 = @intCast(7 - r); // 7 for first (rank 8), 0 for last (rank1)
        var file: u8 = 0;
        for (rank_str) |ch| {
            if (ch >= '1' and ch <= '8') {
                const empty = ch - '0';
                file += empty;
                if (file > 8) return error.InvalidPiecePlacement;
            } else if (Piece.fromChar(ch)) |p| {
                if (file >= 8) return error.InvalidPiecePlacement;
                const sq = Square.make(@enumFromInt(@as(u3, @intCast(file))), @enumFromInt(rank_val));
                // board starts empty, so no collision; but check duplicate
                if (board.pieceAt(sq) != null) return error.InvalidPiecePlacement;
                board.setPiece(sq, p);
                file += 1;
            } else {
                return error.InvalidPiecePlacement;
            }
        }
        if (file != 8) return error.InvalidPiecePlacement;
    }
}

fn parseCastlingField(s: []const u8) FenError!CastlingRights {
    if (s.len == 0) return error.InvalidCastling;
    if (s.len == 1 and s[0] == '-') return CastlingRights.none;
    if (s.len > 4) return error.InvalidCastling;
    var cr = CastlingRights.none;
    for (s) |c| {
        switch (c) {
            'K' => {
                if (cr.white_kingside) return error.InvalidCastling; // duplicate
                cr.white_kingside = true;
            },
            'Q' => {
                if (cr.white_queenside) return error.InvalidCastling;
                cr.white_queenside = true;
            },
            'k' => {
                if (cr.black_kingside) return error.InvalidCastling;
                cr.black_kingside = true;
            },
            'q' => {
                if (cr.black_queenside) return error.InvalidCastling;
                cr.black_queenside = true;
            },
            else => return error.InvalidCastling,
        }
    }
    return cr;
}

fn parseHalfmove(s: []const u8) FenError!u8 {
    if (s.len == 0) return error.InvalidHalfmoveClock;
    for (s) |c| if (c < '0' or c > '9') return error.InvalidHalfmoveClock;
    // No leading zeros unless single '0' ? Allow but not required.
    const v = std.fmt.parseInt(u16, s, 10) catch return error.InvalidHalfmoveClock;
    if (v > 255) return error.InvalidHalfmoveClock;
    return @intCast(v);
}

fn parseFullmove(s: []const u8) FenError!u16 {
    if (s.len == 0) return error.InvalidFullmoveNumber;
    for (s) |c| if (c < '0' or c > '9') return error.InvalidFullmoveNumber;
    const v = std.fmt.parseInt(u32, s, 10) catch return error.InvalidFullmoveNumber;
    if (v == 0 or v > 65535) return error.InvalidFullmoveNumber;
    return @intCast(v);
}

fn checkLegality(board: Board) FenError!void {
    // King count
    const wk = board.pieceCount(.white_king);
    const bk = board.pieceCount(.black_king);
    if (wk != 1 or bk != 1) return error.IllegalPosition;

    // Pawns on first/last rank
    const white_pawns = board.pieces[@intFromEnum(Piece.white_pawn)];
    const black_pawns = board.pieces[@intFromEnum(Piece.black_pawn)];
    const rank1 = Bitboard.rankMask(.@"1");
    const rank8 = Bitboard.rankMask(.@"8");
    if (white_pawns.intersectWith(rank1).isNotEmpty() or
        white_pawns.intersectWith(rank8).isNotEmpty() or
        black_pawns.intersectWith(rank1).isNotEmpty() or
        black_pawns.intersectWith(rank8).isNotEmpty())
    {
        return error.IllegalPosition;
    }

    // Adjacent kings
    const wks = board.kingSquare(.white) orelse return error.IllegalPosition;
    const bks = board.kingSquare(.black) orelse return error.IllegalPosition;
    const wf = @intFromEnum(wks.file());
    const wr = @intFromEnum(wks.rank());
    const bf = @intFromEnum(bks.file());
    const br = @intFromEnum(bks.rank());
    const fd = if (wf > bf) wf - bf else bf - wf;
    const rd = if (wr > br) wr - br else br - wr;
    if (fd <= 1 and rd <= 1) return error.IllegalPosition;

    // Castling rights vs pieces on original squares
    if (board.castling.white_kingside) {
        if (board.pieceAt(.e1) != .white_king or board.pieceAt(.h1) != .white_rook) return error.IllegalPosition;
    }
    if (board.castling.white_queenside) {
        if (board.pieceAt(.e1) != .white_king or board.pieceAt(.a1) != .white_rook) return error.IllegalPosition;
    }
    if (board.castling.black_kingside) {
        if (board.pieceAt(.e8) != .black_king or board.pieceAt(.h8) != .black_rook) return error.IllegalPosition;
    }
    if (board.castling.black_queenside) {
        if (board.pieceAt(.e8) != .black_king or board.pieceAt(.a8) != .black_rook) return error.IllegalPosition;
    }

    // En passant consistency
    if (board.en_passant) |ep| {
        // Must be empty
        if (board.pieceAt(ep) != null) return error.IllegalPosition;
        const ep_rank = ep.rank();
        const ep_file = ep.file();
        // Rank must be 6 if white to move, 3 if black to move
        if (board.side_to_move == .white) {
            if (ep_rank != .@"6") return error.IllegalPosition;
            // Square behind EP (rank 5) must have black pawn that just double-pushed
            const behind = Square.make(ep_file, .@"5");
            if (board.pieceAt(behind) != .black_pawn) return error.IllegalPosition;
            // There must be at least one white pawn adjacent on rank 5 that could capture
            var has_attacker = false;
            if (@intFromEnum(ep_file) > 0) {
                const left = Square.make(@enumFromInt(@intFromEnum(ep_file) - 1), .@"5");
                if (board.pieceAt(left) == .white_pawn) has_attacker = true;
            }
            if (@intFromEnum(ep_file) < 7) {
                const right = Square.make(@enumFromInt(@intFromEnum(ep_file) + 1), .@"5");
                if (board.pieceAt(right) == .white_pawn) has_attacker = true;
            }
            if (!has_attacker) return error.IllegalPosition;
        } else {
            if (ep_rank != .@"3") return error.IllegalPosition;
            const behind = Square.make(ep_file, .@"4");
            if (board.pieceAt(behind) != .white_pawn) return error.IllegalPosition;
            var has_attacker = false;
            if (@intFromEnum(ep_file) > 0) {
                const left = Square.make(@enumFromInt(@intFromEnum(ep_file) - 1), .@"4");
                if (board.pieceAt(left) == .black_pawn) has_attacker = true;
            }
            if (@intFromEnum(ep_file) < 7) {
                const right = Square.make(@enumFromInt(@intFromEnum(ep_file) + 1), .@"4");
                if (board.pieceAt(right) == .black_pawn) has_attacker = true;
            }
            if (!has_attacker) return error.IllegalPosition;
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

test "FEN startpos parse and round-trip" {
    const fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    const b = try parseFen(fen);
    try std.testing.expectEqual(board_mod.Board.startingPosition().castling, b.castling);
    try std.testing.expectEqual(board_mod.Board.startingPosition().side_to_move, b.side_to_move);
    try std.testing.expect(b.en_passant == null);
    try std.testing.expectEqual(@as(u8, 0), b.halfmove_clock);
    try std.testing.expectEqual(@as(u16, 1), b.fullmove_number);
    // round-trip
    var buf: [128]u8 = undefined;
    const out = boardToFen(b, &buf);
    try std.testing.expectEqualStrings(fen, out);
}

test "FEN empty board" {
    const fen = "8/8/8/8/8/8/8/8 w - - 0 1";
    // This is illegal (no kings) — should fail legality
    try std.testing.expectError(error.IllegalPosition, parseFen(fen));
}

test "FEN custom position round-trip" {
    const fen = "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3";
    const b = try parseFen(fen);
    var buf: [128]u8 = undefined;
    const out = boardToFen(b, &buf);
    // parse again and compare boards
    const b2 = try parseFen(out);
    try std.testing.expect(b.eql(b2));
}

test "FEN en passant valid" {
    // After 1.e4 c5 2.e5 d5 — EP d6 capturable by pawn on e5
    // Position with white pawn on e5 adjacent to d6
    const fen = "rnbqkbnr/ppp1pppp/8/3pP3/8/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 2";
    const b = try parseFen(fen);
    try std.testing.expectEqual(Square.d6, b.en_passant.?);
    var buf: [128]u8 = undefined;
    try std.testing.expectEqualStrings(fen, boardToFen(b, &buf));
}

test "FEN en passant illegal rank" {
    const fen = "rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq d5 0 2";
    try std.testing.expectError(error.IllegalPosition, parseFen(fen));
}

test "FEN castling illegal — missing rook" {
    // White K but no rook on h1
    const fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBN1 w K - 0 1";
    try std.testing.expectError(error.IllegalPosition, parseFen(fen));
}

test "FEN illegal pawn on first rank" {
    const fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNP w KQkq - 0 1"; // white pawn on h1 instead of rook? Actually place pawn
    // Let's make pawn on a1
    const fen2 = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/PNBQKBNR w KQkq - 0 1";
    try std.testing.expectError(error.IllegalPosition, parseFen(fen));
    try std.testing.expectError(error.IllegalPosition, parseFen(fen2));
}

test "FEN adjacent kings illegal" {
    const fen = "8/8/8/8/3k4/3K4/8/8 w - - 0 1";
    try std.testing.expectError(error.IllegalPosition, parseFen(fen));
}

test "FEN structural errors" {
    try std.testing.expectError(error.InvalidFen, parseFen("")); // empty
    try std.testing.expectError(error.InvalidFen, parseFen(" rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")); // leading space
    try std.testing.expectError(error.InvalidFen, parseFen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1 ")); // trailing space
    try std.testing.expectError(error.InvalidFen, parseFen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0")); // too few fields
    try std.testing.expectError(error.InvalidPiecePlacement, parseFen("rnbqkbnr/pppppppp/9/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")); // 9 empty
    try std.testing.expectError(error.InvalidFen, parseFen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1 extra")); // too many fields
    try std.testing.expectError(error.InvalidSideToMove, parseFen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR x KQkq - 0 1"));
    try std.testing.expectError(error.InvalidCastling, parseFen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KK - 0 1")); // duplicate
    try std.testing.expectError(error.InvalidEnPassant, parseFen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq i9 0 1"));
    try std.testing.expectError(error.InvalidHalfmoveClock, parseFen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - x 1"));
    try std.testing.expectError(error.InvalidFullmoveNumber, parseFen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 0")); // 0 illegal
}

test "FEN halfmove/fullmove max" {
    const fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 255 65535";
    const b = try parseFen(fen);
    try std.testing.expectEqual(@as(u8, 255), b.halfmove_clock);
    try std.testing.expectEqual(@as(u16, 65535), b.fullmove_number);
    try std.testing.expectError(error.InvalidHalfmoveClock, parseFen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 256 1"));
}

test "FEN startingPosition matches parseFen startpos" {
    const fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    const parsed = try parseFen(fen);
    const expected = board_mod.Board.startingPosition();
    try std.testing.expect(parsed.eql(expected));
}

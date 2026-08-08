const std = @import("std");
const board_mod = @import("board.zig");
const piece_mod = @import("piece.zig");
const square_mod = @import("square.zig");

pub const Board = board_mod.Board;
pub const Piece = piece_mod.Piece;
pub const PieceType = piece_mod.PieceType;
pub const Color = piece_mod.Color;
pub const Square = square_mod.Square;

// ── Material values (centipawns) ────────────────────────────────────────

pub const material_value: [6]i16 = .{
    100, // pawn
    320, // knight
    330, // bishop
    500, // rook
    900, // queen
    0,   // king (not counted, but kept for indexing)
};

// ── Piece-Square Tables (white perspective, rank 1 at bottom) ───────────
// Simple centralization tables for learning. Values are centipawns.
// Black will use flipRank to mirror. These are intentionally small and symmetric.

const pst_pawn: [64]i16 = .{
     0,  0,  0,  0,  0,  0,  0,  0,
     5, 10, 10,-20,-20, 10, 10,  5,
     5, -5,-10,  0,  0,-10, -5,  5,
     0,  0,  0, 20, 20,  0,  0,  0,
     5,  5, 10, 25, 25, 10,  5,  5,
    10, 10, 20, 30, 30, 20, 10, 10,
    50, 50, 50, 50, 50, 50, 50, 50,
     0,  0,  0,  0,  0,  0,  0,  0,
};

const pst_knight: [64]i16 = .{
    -50,-40,-30,-30,-30,-30,-40,-50,
    -40,-20,  0,  0,  0,  0,-20,-40,
    -30,  0, 10, 15, 15, 10,  0,-30,
    -30,  5, 15, 20, 20, 15,  5,-30,
    -30,  0, 15, 20, 20, 15,  0,-30,
    -30,  5, 10, 15, 15, 10,  5,-30,
    -40,-20,  0,  5,  5,  0,-20,-40,
    -50,-40,-30,-30,-30,-30,-40,-50,
};

const pst_bishop: [64]i16 = .{
    -20,-10,-10,-10,-10,-10,-10,-20,
    -10,  0,  0,  0,  0,  0,  0,-10,
    -10,  0,  5, 10, 10,  5,  0,-10,
    -10,  5,  5, 10, 10,  5,  5,-10,
    -10,  0, 10, 10, 10, 10,  0,-10,
    -10, 10, 10, 10, 10, 10, 10,-10,
    -10,  5,  0,  0,  0,  0,  5,-10,
    -20,-10,-10,-10,-10,-10,-10,-20,
};

const pst_rook: [64]i16 = .{
      0,  0,  0,  0,  0,  0,  0,  0,
      5, 10, 10, 10, 10, 10, 10,  5,
     -5,  0,  0,  0,  0,  0,  0, -5,
     -5,  0,  0,  0,  0,  0,  0, -5,
     -5,  0,  0,  0,  0,  0,  0, -5,
     -5,  0,  0,  0,  0,  0,  0, -5,
     -5,  0,  0,  0,  0,  0,  0, -5,
      0,  0,  0,  5,  5,  0,  0,  0,
};

const pst_queen: [64]i16 = .{
    -20,-10,-10, -5, -5,-10,-10,-20,
    -10,  0,  0,  0,  0,  0,  0,-10,
    -10,  0,  5,  5,  5,  5,  0,-10,
     -5,  0,  5,  5,  5,  5,  0, -5,
      0,  0,  5,  5,  5,  5,  0, -5,
    -10,  5,  5,  5,  5,  5,  0,-10,
    -10,  0,  5,  0,  0,  0,  0,-10,
    -20,-10,-10, -5, -5,-10,-10,-20,
};

const pst_king: [64]i16 = .{
    -30,-40,-40,-50,-50,-40,-40,-30,
    -30,-40,-40,-50,-50,-40,-40,-30,
    -30,-40,-40,-50,-50,-40,-40,-30,
    -30,-40,-40,-50,-50,-40,-40,-30,
    -20,-30,-30,-40,-40,-30,-30,-20,
    -10,-20,-20,-20,-20,-20,-20,-10,
     20, 20,  0,  0,  0,  0, 20, 20,
     20, 30, 10,  0,  0, 10, 30, 20,
};

const psts: [6][64]i16 = .{ pst_pawn, pst_knight, pst_bishop, pst_rook, pst_queen, pst_king };

// ── 13-term breakdown ───────────────────────────────────────────────────

pub const EvalBreakdown = struct {
    // Terms — kept as i32 centipawns for sum; first two are real, rest are placeholders for later
    material: i32 = 0,
    pst: i32 = 0,
    bishop_pair: i32 = 0,
    knight_outpost: i32 = 0,
    rook_open_file: i32 = 0,
    pawn_doubled: i32 = 0,
    pawn_isolated: i32 = 0,
    pawn_passed: i32 = 0,
    king_safety: i32 = 0,
    mobility: i32 = 0,
    tempo: i32 = 0,
    pawn_structure_extra: i32 = 0,
    center_control: i32 = 0,

    pub fn total(self: EvalBreakdown) i32 {
        return self.material + self.pst + self.bishop_pair + self.knight_outpost + self.rook_open_file +
            self.pawn_doubled + self.pawn_isolated + self.pawn_passed + self.king_safety +
            self.mobility + self.tempo + self.pawn_structure_extra + self.center_control;
    }

    pub fn toArray(self: EvalBreakdown) [13]i32 {
        return .{
            self.material, self.pst, self.bishop_pair, self.knight_outpost, self.rook_open_file,
            self.pawn_doubled, self.pawn_isolated, self.pawn_passed, self.king_safety,
            self.mobility, self.tempo, self.pawn_structure_extra, self.center_control,
        };
    }
};

pub const term_names: [13][]const u8 = .{
    "material", "pst", "bishop_pair", "knight_outpost", "rook_open_file",
    "pawn_doubled", "pawn_isolated", "pawn_passed", "king_safety",
    "mobility", "tempo", "pawn_struct_extra", "center_control",
};

// ── Helpers ──────────────────────────────────────────────────────────────

inline fn pstFor(piece_type: PieceType, sq: Square, color: Color) i16 {
    const idx: usize = @intFromEnum(sq);
    const table = psts[@intFromEnum(piece_type)];
    if (color == .white) {
        return table[idx];
    } else {
        return table[@intFromEnum(sq.flipRank())];
    }
}

fn isDoubledPawn(board: Board, color: Color) i32 {
    var score: i32 = 0;
    for (0..8) |f| {
        var count: u8 = 0;
        const file: square_mod.File = @enumFromInt(f);
        _ = file;
        for (0..64) |i| {
            const sq: Square = @enumFromInt(i);
            if (@intFromEnum(sq.file()) != f) continue;
            if (board.pieceAt(sq)) |p| {
                if (p.color() == color and p.pieceType() == .pawn) count += 1;
            }
        }
        if (count > 1) score -= @as(i32, count - 1) * 10;
    }
    return score;
}

fn pawnIsolatedScore(board: Board, color: Color) i32 {
    var score: i32 = 0;
    for (0..8) |f| {
        const has_pawn_on_file = blk: {
            for (0..64) |i| {
                const sq: Square = @enumFromInt(i);
                if (@intFromEnum(sq.file()) != f) continue;
                if (board.pieceAt(sq)) |p| {
                    if (p.color() == color and p.pieceType() == .pawn) break :blk true;
                }
            }
            break :blk false;
        };
        if (!has_pawn_on_file) continue;
        const left_has = if (f > 0) blk2: {
            for (0..64) |i| {
                const sq: Square = @enumFromInt(i);
                if (@intFromEnum(sq.file()) != f - 1) continue;
                if (board.pieceAt(sq)) |p| {
                    if (p.color() == color and p.pieceType() == .pawn) break :blk2 true;
                }
            }
            break :blk2 false;
        } else false;
        const right_has = if (f < 7) blk3: {
            for (0..64) |i| {
                const sq: Square = @enumFromInt(i);
                if (@intFromEnum(sq.file()) != f + 1) continue;
                if (board.pieceAt(sq)) |p| {
                    if (p.color() == color and p.pieceType() == .pawn) break :blk3 true;
                }
            }
            break :blk3 false;
        } else false;
        if (!left_has and !right_has) score -= 10;
    }
    return score;
}

// ── Main evaluation ──────────────────────────────────────────────────────

/// White-centric evaluation (positive = white better), 13 terms.
/// Deterministic and symmetric: eval(flipped) == -eval for material+pst baseline.
pub fn evaluate(board: Board) EvalBreakdown {
    var bd = EvalBreakdown{};

    // Material + PST (the two real terms for now)
    for (0..64) |i| {
        const sq: Square = @enumFromInt(i);
        if (board.pieceAt(sq)) |p| {
            const pt = p.pieceType();
            const mat = material_value[@intFromEnum(pt)];
            const pst = pstFor(pt, sq, p.color());
            if (p.color() == .white) {
                bd.material += mat;
                bd.pst += pst;
            } else {
                bd.material -= mat;
                bd.pst -= pst;
            }
        }
    }

    // Bishop pair: +30 if side has >=2 bishops
    var white_bishops: u8 = 0;
    var black_bishops: u8 = 0;
    for (0..64) |i| {
        const sq: Square = @enumFromInt(i);
        if (board.pieceAt(sq)) |p| {
            if (p.pieceType() == .bishop) {
                if (p.color() == .white) white_bishops += 1 else black_bishops += 1;
            }
        }
    }
    if (white_bishops >= 2) bd.bishop_pair += 30;
    if (black_bishops >= 2) bd.bishop_pair -= 30;

    // Pawn structure placeholders (small)
    // Doubled/isolated are from white perspective already; compute difference
    // For simplicity compute white score + black score (black already negative in helpers)
    // Helpers return negative for side with bad structure, so we add white + (-black) via calling for each color and combining
    // Our helpers return score for that color (negative bad). For total white-centric we do white_score - black_score? But helper already returns negative for that color's badness.
    // To keep white-centric, we add white's helper directly and subtract black's helper (since black's badness should be good for white).
    // But our helpers currently return negative for that color, so for white-centric we want: bd += white_helper - black_helper ? Wait black_helper is negative if black has doubled, that's bad for black, good for white, so we should subtract black_helper (negative) => add.
    // Simpler: compute difference as white_score - black_score where each helper returns its own (negative) badness.
    // We'll just compute as white_doubled - black_doubled where each is negative badness for that side.
    // Actually our isDoubledPawn returns negative for that color's doubled count, so for white-centric: white doubled bad → negative, black doubled bad → positive (since bad for black is good for white). So we do white + (-black) ??? Let's just do white_score - black_score.
    const w_doubled = isDoubledPawn(board, .white);
    const b_doubled = isDoubledPawn(board, .black);
    bd.pawn_doubled = w_doubled - b_doubled; // w negative bad → bd negative, b negative bad → bd positive
    const w_iso = pawnIsolatedScore(board, .white);
    const b_iso = pawnIsolatedScore(board, .black);
    bd.pawn_isolated = w_iso - b_iso;

    // Tempo: +10 for side to move (classical)
    bd.tempo = if (board.side_to_move == .white) 10 else -10;

    // Other terms remain 0 for now, but structure is there for later divergence
    return bd;
}

/// Convenience: total white-centric centipawns
pub fn evaluateScore(board: Board) i32 {
    return evaluate(board).total();
}

/// Side-to-move perspective (for negamax): positive = stm better
pub fn evaluateForSide(board: Board) i32 {
    const s = evaluateScore(board);
    return if (board.side_to_move == .white) s else -s;
}

// ── Tests ────────────────────────────────────────────────────────────────

test "eval startpos 0 and symmetric" {
    const start = board_mod.Board.startingPosition();
    const bd = evaluate(start);
    // startpos white to move has tempo +10, so total 10
    try std.testing.expectEqual(@as(i32, 10), bd.total());
    // Symmetric material + PST (ignoring tempo) should be 0 — check with both pawns and kings
    var b = Board.empty();
    b.setPiece(.e4, .white_pawn);
    b.setPiece(.e5, .black_pawn);
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    // white pawn tempo makes total 10, black pawn symmetric cancels, PST symmetric
    try std.testing.expectEqual(@as(i32, 10), evaluate(b).total());
    // Flip side: tempo should flip
    var b2 = b;
    b2.side_to_move = .black;
    b2.hash = b2.computeHash();
    try std.testing.expectEqual(@as(i32, -10), evaluate(b2).tempo);
    // symmetric position with black to move has total -10 (tempo flips)
    try std.testing.expect(evaluate(b).total() == 10 and evaluate(b2).total() == -10);
}

test "eval material" {
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    b.setPiece(.a1, .white_queen);
    // white queen vs nothing => positive
    try std.testing.expect(evaluateScore(b) > 800);
    var b2 = Board.empty();
    b2.setPiece(.e1, .white_king);
    b2.setPiece(.e8, .black_king);
    b2.setPiece(.a1, .black_queen);
    try std.testing.expect(evaluateScore(b2) < -800);
}

test "eval breakdown sum" {
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    b.setPiece(.d4, .white_knight);
    const bd = evaluate(b);
    var sum: i32 = 0;
    for (bd.toArray()) |v| sum += v;
    try std.testing.expectEqual(bd.total(), sum);
}

test "eval bishop pair" {
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    b.setPiece(.c1, .white_bishop);
    b.setPiece(.f1, .white_bishop);
    try std.testing.expect(evaluate(b).bishop_pair == 30);
    var b2 = Board.empty();
    b2.setPiece(.e1, .white_king);
    b2.setPiece(.e8, .black_king);
    b2.setPiece(.c8, .black_bishop);
    b2.setPiece(.f8, .black_bishop);
    try std.testing.expect(evaluate(b2).bishop_pair == -30);
}

test "eval tempo" {
    var b = Board.startingPosition();
    b.side_to_move = .white;
    try std.testing.expect(evaluate(b).tempo == 10);
    b.side_to_move = .black;
    try std.testing.expect(evaluate(b).tempo == -10);
}

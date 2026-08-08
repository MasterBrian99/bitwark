const std = @import("std");
const board_mod = @import("board.zig");
const piece_mod = @import("piece.zig");

pub const Board = board_mod.Board;
pub const Piece = piece_mod.Piece;

pub const GamePhase = enum { opening, middlegame, endgame };

// Stockfish-like phase: Q=4, R=2, N=1, B=1 both colors → 0..24
// Reusable for tapered eval later.
pub fn phaseValue(board: Board) u8 {
    var p: u8 = 0;
    p += board.pieceCount(.white_queen) * 4;
    p += board.pieceCount(.black_queen) * 4;
    p += board.pieceCount(.white_rook) * 2;
    p += board.pieceCount(.black_rook) * 2;
    p += board.pieceCount(.white_knight);
    p += board.pieceCount(.black_knight);
    p += board.pieceCount(.white_bishop);
    p += board.pieceCount(.black_bishop);
    return p; // max 24
}

pub fn classify(board: Board) GamePhase {
    const ph = phaseValue(board);
    if (ph >= 18) return .opening;
    if (ph >= 6) return .middlegame;
    return .endgame;
}

// Alternative helper: isQueenless endgame refinement
pub fn isEndgameQueenless(board: Board) bool {
    return board.pieceCount(.white_queen) == 0 and board.pieceCount(.black_queen) == 0 and phaseValue(board) <= 8;
}

test "phase startpos opening" {
    const b = Board.startingPosition();
    try std.testing.expectEqual(@as(u8, 24), phaseValue(b));
    try std.testing.expectEqual(GamePhase.opening, classify(b));
}

test "phase middlegame" {
    // Remove some majors: Q+R vs Q+R → phase = 4*2 +2*2=12 → middlegame
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    b.setPiece(.d1, .white_queen);
    b.setPiece(.d8, .black_queen);
    b.setPiece(.a1, .white_rook);
    b.setPiece(.a8, .black_rook);
    try std.testing.expectEqual(@as(u8, 12), phaseValue(b));
    try std.testing.expectEqual(GamePhase.middlegame, classify(b));
}

test "phase endgame" {
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    b.setPiece(.a2, .white_pawn);
    b.setPiece(.a7, .black_pawn);
    try std.testing.expectEqual(GamePhase.endgame, classify(b));
    try std.testing.expect(isEndgameQueenless(b));
}

test "phase thresholds" {
    // Exactly 18 → opening, 17 → middlegame, 5 → endgame
    var b18 = Board.empty();
    b18.setPiece(.e1, .white_king);
    b18.setPiece(.e8, .black_king);
    // 24 - 6 =18: remove one Q (4) + one R (2) from startpos equivalent
    // Build 18: Q*2=8, R*2=4, N*2=2, B*4=4 → total 18
    b18.setPiece(.d1, .white_queen);
    b18.setPiece(.d8, .black_queen);
    b18.setPiece(.a1, .white_rook);
    b18.setPiece(.a8, .black_rook);
    b18.setPiece(.b1, .white_knight);
    b18.setPiece(.b8, .black_knight);
    b18.setPiece(.c1, .white_bishop);
    b18.setPiece(.c8, .black_bishop);
    b18.setPiece(.f1, .white_bishop);
    b18.setPiece(.f8, .black_bishop); // 4*2 +2*2 +1*2 +1*4 =8+4+2+4=18
    try std.testing.expectEqual(GamePhase.opening, classify(b18));

    var b5 = Board.empty();
    b5.setPiece(.e1, .white_king);
    b5.setPiece(.e8, .black_king);
    b5.setPiece(.a1, .white_rook);
    b5.setPiece(.b1, .white_knight);
    b5.setPiece(.c1, .white_bishop); // 2+1+1=4
    b5.setPiece(.a8, .black_rook); // +2 =6 → middlegame
    try std.testing.expectEqual(GamePhase.middlegame, classify(b5));
    // remove black rook → 4 → endgame
    _ = b5.removePiece(.a8);
    b5.hash = b5.computeHash();
    try std.testing.expectEqual(GamePhase.endgame, classify(b5));
}

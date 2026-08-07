pub const piece = @import("piece.zig");
pub const square = @import("square.zig");
pub const bitboard = @import("bitboard.zig");
pub const castling = @import("castling.zig");
pub const board = @import("board.zig");
pub const fen = @import("fen.zig");
pub const move = @import("move.zig");
pub const movelist = @import("movelist.zig");
pub const attacks = @import("attacks.zig");
pub const movegen = @import("movegen.zig");

// Re-export commonly used types at core root for convenience
pub const Color = piece.Color;
pub const Piece = piece.Piece;
pub const PieceType = piece.PieceType;
pub const Square = square.Square;
pub const File = square.File;
pub const Rank = square.Rank;
pub const Bitboard = bitboard.Bitboard;
pub const CastlingRights = castling.CastlingRights;
pub const Board = board.Board;
pub const Move = move.Move;
pub const MoveList = movelist.MoveList;

test {
    @import("std").testing.refAllDecls(@This());
    // Pull in sub-module tests
    _ = @import("piece.zig");
    _ = @import("square.zig");
    _ = @import("bitboard.zig");
    _ = @import("castling.zig");
    _ = @import("board.zig");
    _ = @import("fen.zig");
    _ = @import("move.zig");
    _ = @import("movelist.zig");
    _ = @import("attacks.zig");
    _ = @import("movegen.zig");
}

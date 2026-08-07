const std = @import("std");
const piece_mod = @import("piece.zig");
const square_mod = @import("square.zig");
const bitboard_mod = @import("bitboard.zig");
const board_mod = @import("board.zig");
const move_mod = @import("move.zig");
const movelist_mod = @import("movelist.zig");
const attacks_mod = @import("attacks.zig");

pub const Board = board_mod.Board;
pub const Move = move_mod.Move;
pub const MoveList = movelist_mod.MoveList;
pub const Square = square_mod.Square;
pub const File = square_mod.File;
pub const Rank = square_mod.Rank;
pub const Color = piece_mod.Color;
pub const Piece = piece_mod.Piece;
pub const PieceType = piece_mod.PieceType;
pub const Bitboard = bitboard_mod.Bitboard;

// ── Public API ───────────────────────────────────────────────────────────

pub fn generatePseudoLegal(board: Board, list: *MoveList) void {
    list.clear();
    const stm = board.side_to_move;
    const us = board.occupancyFor(stm);
    const them = board.occupancyFor(stm.opposite());
    const occupied = board.occupied_all;
    const our_pawns = board.pieces[@intFromEnum(if (stm == .white) Piece.white_pawn else Piece.black_pawn)];
    const our_knights = board.pieces[@intFromEnum(if (stm == .white) Piece.white_knight else Piece.black_knight)];
    const our_bishops = board.pieces[@intFromEnum(if (stm == .white) Piece.white_bishop else Piece.black_bishop)];
    const our_rooks = board.pieces[@intFromEnum(if (stm == .white) Piece.white_rook else Piece.black_rook)];
    const our_queens = board.pieces[@intFromEnum(if (stm == .white) Piece.white_queen else Piece.black_queen)];
    const our_king = board.pieces[@intFromEnum(if (stm == .white) Piece.white_king else Piece.black_king)];

    // Pawns
    generatePawnMoves(board, our_pawns, stm, us, them, occupied, list);
    // Knights
    generateLeaperMoves(our_knights, us, them, attacks_mod.knightAttacks, list);
    // Bishops
    generateSlidingMoves(board, our_bishops, stm, us, them, occupied, true, false, list);
    // Rooks
    generateSlidingMoves(board, our_rooks, stm, us, them, occupied, false, true, list);
    // Queens (both)
    generateSlidingMoves(board, our_queens, stm, us, them, occupied, true, true, list);
    // King
    generateKingMoves(board, our_king, stm, us, them, list);
}

pub fn generateLegal(board: Board, list: *MoveList) void {
    var pseudo = MoveList{};
    generatePseudoLegal(board, &pseudo);
    list.clear();
    for (pseudo.moves[0..pseudo.len]) |m| {
        var copy = board;
        applyMove(&copy, m);
        // After move, side to move flips? For legality we check if the moving side's king is left in check.
        // applyMove flips side_to_move, but we want to check the king of the mover (original stm).
        // So we check if mover's king is attacked by opponent (which is now side_to_move).
        const mover = board.side_to_move;
        const king_sq = copy.kingSquare(mover) orelse continue; // should exist
        if (!attacks_mod.isSquareAttacked(copy, king_sq, copy.side_to_move)) {
            list.push(m);
        }
    }
}

// ── Pawn generation ──────────────────────────────────────────────────────

fn generatePawnMoves(board: Board, pawns: Bitboard, stm: Color, us: Bitboard, them: Bitboard, occupied: Bitboard, list: *MoveList) void {
    _ = us;
    _ = occupied;
    _ = them;
    const dir: i8 = if (stm == .white) 1 else -1;
    const start_rank: Rank = if (stm == .white) .@"2" else .@"7";
    const promo_rank: Rank = if (stm == .white) .@"8" else .@"1";
    var it = pawns.iterator();
    while (it.next()) |sq| {
        const f: i8 = @intFromEnum(sq.file());
        const r: i8 = @intFromEnum(sq.rank());
        const nr = r + dir;
        if (nr < 0 or nr >= 8) continue;
        const one_forward = Square.make(@enumFromInt(@as(u3, @intCast(f))), @enumFromInt(@as(u3, @intCast(nr))));
        // Single push
        if (board.pieceAt(one_forward) == null) {
            if (@as(Rank, @enumFromInt(@as(u3, @intCast(nr)))) == promo_rank) {
                for ([_]PieceType{ .queen, .rook, .bishop, .knight }) |pt| {
                    list.push(.{ .from = sq, .to = one_forward, .promotion = pt, .is_capture = false });
                }
            } else {
                list.push(.{ .from = sq, .to = one_forward });
                // Double push
                if (sq.rank() == start_rank) {
                    const nr2 = r + dir * 2;
                    const two_forward = Square.make(@enumFromInt(@as(u3, @intCast(f))), @enumFromInt(@as(u3, @intCast(nr2))));
                    if (board.pieceAt(two_forward) == null) {
                        list.push(.{ .from = sq, .to = two_forward, .is_double_push = true });
                    }
                }
            }
        }
        // Captures (including en passant)
        for ([_]i8{ -1, 1 }) |df| {
            const nf = f + df;
            if (nf < 0 or nf >= 8) continue;
            const cap_sq = Square.make(@enumFromInt(@as(u3, @intCast(nf))), @enumFromInt(@as(u3, @intCast(nr))));
            const target = board.pieceAt(cap_sq);
            const is_ep = board.en_passant != null and board.en_passant.? == cap_sq;
            if (target != null and target.?.color() != stm) {
                // normal capture
                if (@as(Rank, @enumFromInt(@as(u3, @intCast(nr)))) == promo_rank) {
                    for ([_]PieceType{ .queen, .rook, .bishop, .knight }) |pt| {
                        list.push(.{ .from = sq, .to = cap_sq, .promotion = pt, .is_capture = true });
                    }
                } else {
                    list.push(.{ .from = sq, .to = cap_sq, .is_capture = true });
                }
            } else if (is_ep) {
                // en passant capture — only if there is a pawn to capture behind?
                // FEN legality already ensures capturer exists, but we still generate.
                list.push(.{ .from = sq, .to = cap_sq, .is_capture = true, .is_en_passant = true });
            }
        }
        _ = occupied; // not needed separately (we used pieceAt)
    }
}

// ── Leapers (knight) ────────────────────────────────────────────────────

fn generateLeaperMoves(pieces: Bitboard, us: Bitboard, them: Bitboard, table: [64]Bitboard, list: *MoveList) void {
    var it = pieces.iterator();
    while (it.next()) |sq| {
        var attacks = table[@intFromEnum(sq)];
        // remove own pieces
        attacks = attacks.without(us);
        var ait = attacks.iterator();
        while (ait.next()) |to| {
            const is_cap = them.contains(to);
            list.push(.{ .from = sq, .to = to, .is_capture = is_cap });
        }
    }
}

// ── Sliding ─────────────────────────────────────────────────────────────

fn generateSlidingMoves(board: Board, pieces: Bitboard, stm: Color, us: Bitboard, them: Bitboard, occupied: Bitboard, is_bishop: bool, is_rook: bool, list: *MoveList) void {
    _ = stm;
    var it = pieces.iterator();
    while (it.next()) |sq| {
        var attacks: Bitboard = undefined;
        if (is_bishop and is_rook) {
            attacks = attacks_mod.queenAttacks(sq, occupied);
        } else if (is_bishop) {
            attacks = attacks_mod.bishopAttacks(sq, occupied);
        } else {
            attacks = attacks_mod.rookAttacks(sq, occupied);
        }
        attacks = attacks.without(us);
        var ait = attacks.iterator();
        while (ait.next()) |to| {
            const is_cap = them.contains(to);
            // Note: sliding attack already blocked by occupancy, so this is pseudo-legal
            // We rely on legal filter to remove pinned illegal moves later (for now)
            _ = board;
            list.push(.{ .from = sq, .to = to, .is_capture = is_cap });
        }
    }
}

// ── King ────────────────────────────────────────────────────────────────

fn generateKingMoves(board: Board, king_bb: Bitboard, stm: Color, us: Bitboard, them: Bitboard, list: *MoveList) void {
    var it = king_bb.iterator();
    while (it.next()) |sq| {
        var attacks = attacks_mod.kingAttacks[@intFromEnum(sq)];
        attacks = attacks.without(us);
        var ait = attacks.iterator();
        while (ait.next()) |to| {
            // Don't allow capturing own, and don't allow moving into check? That would be illegal,
            // but we generate pseudolegal then filter, so allow for now.
            const is_cap = them.contains(to);
            list.push(.{ .from = sq, .to = to, .is_capture = is_cap });
        }
        // Castling
        if (stm == .white and sq == .e1) {
            // Kingside: e1->g1 if K, f1/g1 empty, e1/f1/g1 not attacked
            if (board.castling.white_kingside and
                board.pieceAt(.f1) == null and board.pieceAt(.g1) == null)
            {
                if (!attacks_mod.isSquareAttacked(board, .e1, .black) and
                    !attacks_mod.isSquareAttacked(board, .f1, .black) and
                    !attacks_mod.isSquareAttacked(board, .g1, .black))
                {
                    list.push(.{ .from = .e1, .to = .g1, .is_castle = true });
                }
            }
            // Queenside: e1->c1 if Q, b1/c1/d1 empty? require c1/d1 empty, b1 can be occupied by some interpretations but we require empty for safety
            if (board.castling.white_queenside and
                board.pieceAt(.b1) == null and board.pieceAt(.c1) == null and board.pieceAt(.d1) == null)
            {
                if (!attacks_mod.isSquareAttacked(board, .e1, .black) and
                    !attacks_mod.isSquareAttacked(board, .d1, .black) and
                    !attacks_mod.isSquareAttacked(board, .c1, .black))
                {
                    list.push(.{ .from = .e1, .to = .c1, .is_castle = true });
                }
            }
        } else if (stm == .black and sq == .e8) {
            if (board.castling.black_kingside and
                board.pieceAt(.f8) == null and board.pieceAt(.g8) == null)
            {
                if (!attacks_mod.isSquareAttacked(board, .e8, .white) and
                    !attacks_mod.isSquareAttacked(board, .f8, .white) and
                    !attacks_mod.isSquareAttacked(board, .g8, .white))
                {
                    list.push(.{ .from = .e8, .to = .g8, .is_castle = true });
                }
            }
            if (board.castling.black_queenside and
                board.pieceAt(.b8) == null and board.pieceAt(.c8) == null and board.pieceAt(.d8) == null)
            {
                if (!attacks_mod.isSquareAttacked(board, .e8, .white) and
                    !attacks_mod.isSquareAttacked(board, .d8, .white) and
                    !attacks_mod.isSquareAttacked(board, .c8, .white))
                {
                    list.push(.{ .from = .e8, .to = .c8, .is_castle = true });
                }
            }
        }
    }
}

// ── Make move on copy (for legality filtering) ────────────────────────

fn applyMove(board: *Board, m: Move) void {
    const stm = board.side_to_move;
    const moving = board.pieceAt(m.from) orelse return;
    // Remove from
    _ = board.removePiece(m.from);
    // Handle capture
    if (m.is_en_passant) {
        // captured pawn is behind `to`
        const cap_rank: Rank = if (stm == .white) .@"5" else .@"4";
        const cap_sq = Square.make(m.to.file(), cap_rank);
        _ = board.removePiece(cap_sq);
    } else if (m.is_capture) {
        _ = board.removePiece(m.to);
    }
    // Handle promotion
    if (m.promotion) |pt| {
        const promo_piece = Piece.make(stm, pt);
        board.setPiece(m.to, promo_piece);
    } else {
        board.setPiece(m.to, moving);
    }
    // Handle castling rook move
    if (m.is_castle) {
        if (m.from == .e1 and m.to == .g1) {
            _ = board.removePiece(.h1);
            board.setPiece(.f1, .white_rook);
        } else if (m.from == .e1 and m.to == .c1) {
            _ = board.removePiece(.a1);
            board.setPiece(.d1, .white_rook);
        } else if (m.from == .e8 and m.to == .g8) {
            _ = board.removePiece(.h8);
            board.setPiece(.f8, .black_rook);
        } else if (m.from == .e8 and m.to == .c8) {
            _ = board.removePiece(.a8);
            board.setPiece(.d8, .black_rook);
        }
    }
    // Update castling rights (simple: if king or rook moves/captured)
    // For legality filtering we only need enough to not break next isSquareAttacked,
    // but we also maintain rights for future moves in search. So update conservatively.
    if (moving.pieceType() == .king) {
        if (stm == .white) {
            board.castling.white_kingside = false;
            board.castling.white_queenside = false;
        } else {
            board.castling.black_kingside = false;
            board.castling.black_queenside = false;
        }
    } else if (moving.pieceType() == .rook) {
        if (m.from == .a1) board.castling.white_queenside = false;
        if (m.from == .h1) board.castling.white_kingside = false;
        if (m.from == .a8) board.castling.black_queenside = false;
        if (m.from == .h8) board.castling.black_kingside = false;
    }
    // If capture removed rook on origin, also clear rights
    if (m.is_capture and !m.is_en_passant) {
        if (m.to == .a1) board.castling.white_queenside = false;
        if (m.to == .h1) board.castling.white_kingside = false;
        if (m.to == .a8) board.castling.black_queenside = false;
        if (m.to == .h8) board.castling.black_kingside = false;
    }
    // En passant square: set only on double push, otherwise clear
    if (m.is_double_push) {
        const ep_rank: Rank = if (stm == .white) .@"3" else .@"6";
        board.en_passant = Square.make(m.to.file(), ep_rank);
        // Actually EP square is the square behind the double-pushed pawn's destination?
        // Wait spec: after double push, EP is square behind pawn's destination? No, it's the square the pawn passed over.
        // For double push e2->e4, EP should be e3 (rank 3). Our m.to is e4, file e, rank 3 is e3. That's not m.to.file with ep_rank? m.to is e4 (rank 4), EP is e3 (rank 3). Our code uses m.to.file with ep_rank 3 -> e3 correct.
        // For black double push e7->e5, EP is e6. m.to is e5 rank 5, ep_rank 6 -> e6 correct.
    } else {
        board.en_passant = null;
    }
    // Halfmove: reset on pawn move or capture, else inc
    if (moving.pieceType() == .pawn or m.is_capture) {
        board.halfmove_clock = 0;
    } else {
        board.halfmove_clock +|= 1;
    }
    // Fullmove: inc after black move
    if (stm == .black) board.fullmove_number +|= 1;
    // Flip side
    board.side_to_move = stm.opposite();
}

// ── Tests ────────────────────────────────────────────────────────────────

test "startpos pseudo and legal count 20" {
    const board = board_mod.Board.startingPosition();
    var pseudo = MoveList{};
    generatePseudoLegal(board, &pseudo);
    try std.testing.expectEqual(@as(usize, 20), pseudo.len);
    var legal = MoveList{};
    generateLegal(board, &legal);
    try std.testing.expectEqual(@as(usize, 20), legal.len);
    // contains e2e4
    try std.testing.expect(legal.containsUci("e2e4"));
    try std.testing.expect(legal.containsUci("g1f3"));
}

test "kiwipete-like position move count sanity" {
    // Position after 1.d4 Nf6 2.c4 e6 — a bit more open
    const fen = "rnbqkbnr/pppp1ppp/4p3/8/2PP4/8/PP2PPPP/RNBQKBNR w KQkq - 0 2";
    const b = try @import("fen.zig").parseFen(fen);
    var legal = MoveList{};
    generateLegal(b, &legal);
    // Just check non-zero and reasonable
    try std.testing.expect(legal.len > 20);
    try std.testing.expect(legal.len < 50);
}

test "pawn promotion generation" {
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    b.setPiece(.a7, .white_pawn);
    b.side_to_move = .white;
    var list = MoveList{};
    generatePseudoLegal(b, &list);
    // pawn on a7 should have 4 promotions to a8 (quiet) + maybe captures if pieces on b8
    var promo_count: usize = 0;
    for (list.moves[0..list.len]) |m| {
        if (m.promotion != null) promo_count += 1;
    }
    try std.testing.expectEqual(@as(usize, 4), promo_count);
    // Add black rook on b8 to test capture promotions
    b.setPiece(.b8, .black_rook);
    list.clear();
    generatePseudoLegal(b, &list);
    promo_count = 0;
    for (list.moves[0..list.len]) |m| {
        if (m.promotion != null) promo_count += 1;
    }
    try std.testing.expectEqual(@as(usize, 8), promo_count); // 4 quiet + 4 capture
}

test "castling generation" {
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.a1, .white_rook);
    b.setPiece(.h1, .white_rook);
    b.setPiece(.e8, .black_king);
    b.castling = .{ .white_kingside = true, .white_queenside = true };
    b.side_to_move = .white;
    var list = MoveList{};
    generateLegal(b, &list);
    try std.testing.expect(list.containsUci("e1g1"));
    try std.testing.expect(list.containsUci("e1c1"));
    // Block f1
    b.setPiece(.f1, .white_bishop);
    list.clear();
    generateLegal(b, &list);
    try std.testing.expect(!list.containsUci("e1g1"));
    try std.testing.expect(list.containsUci("e1c1"));
}

test "en passant generation" {
    // Position with EP d6 capturable by e5 pawn
    const fen = "rnbqkbnr/ppp1pppp/8/3pP3/8/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 2";
    const b = try @import("fen.zig").parseFen(fen);
    var list = MoveList{};
    generateLegal(b, &list);
    try std.testing.expect(list.containsUci("e5d6"));
}

test "pin — illegal rook move filtered" {
    // White king e1, white rook e2 pinned by black rook e8 against king
    var b = Board.empty();
    b.setPiece(.e1, .white_king);
    b.setPiece(.e2, .white_rook);
    b.setPiece(.e8, .black_rook);
    b.setPiece(.a8, .black_king);
    b.side_to_move = .white;
    var list = MoveList{};
    generateLegal(b, &list);
    // Rook on e2 should only be able to stay on e-file (including capture e8) — not move off file
    for (list.moves[0..list.len]) |m| {
        if (m.from == .e2) {
            try std.testing.expectEqual(File.e, m.to.file());
        }
    }
}

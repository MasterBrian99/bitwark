const std = @import("std");
const square_mod = @import("square.zig");
const bitboard_mod = @import("bitboard.zig");
const piece_mod = @import("piece.zig");
const board_mod = @import("board.zig");

pub const Square = square_mod.Square;
pub const File = square_mod.File;
pub const Rank = square_mod.Rank;
pub const Bitboard = bitboard_mod.Bitboard;
pub const PieceType = piece_mod.PieceType;
pub const Color = piece_mod.Color;
pub const Board = board_mod.Board;

// ── Leaper tables (comptime) ───────────────────────────────────────────

pub const knightAttacks: [64]Bitboard = blk: {
    @setEvalBranchQuota(10000);
    var tbl: [64]Bitboard = undefined;
    for (0..64) |i| {
        const sq: Square = @enumFromInt(i);
        const f: i8 = @intFromEnum(sq.file());
        const r: i8 = @intFromEnum(sq.rank());
        var bb = Bitboard.empty;
        const offs = [_][2]i8{ .{ -1, -2 }, .{ 1, -2 }, .{ -2, -1 }, .{ 2, -1 }, .{ -2, 1 }, .{ 2, 1 }, .{ -1, 2 }, .{ 1, 2 } };
        for (offs) |o| {
            const nf = f + o[0];
            const nr = r + o[1];
            if (nf >= 0 and nf < 8 and nr >= 0 and nr < 8) {
                const nsq = Square.make(@enumFromInt(@as(u3, @intCast(nf))), @enumFromInt(@as(u3, @intCast(nr))));
                bb.set(nsq);
            }
        }
        tbl[i] = bb;
    }
    break :blk tbl;
};

pub const kingAttacks: [64]Bitboard = blk: {
    @setEvalBranchQuota(10000);
    var tbl: [64]Bitboard = undefined;
    for (0..64) |i| {
        const sq: Square = @enumFromInt(i);
        const f: i8 = @intFromEnum(sq.file());
        const r: i8 = @intFromEnum(sq.rank());
        var bb = Bitboard.empty;
        var df: i8 = -1;
        while (df <= 1) : (df += 1) {
            var dr: i8 = -1;
            while (dr <= 1) : (dr += 1) {
                if (df == 0 and dr == 0) continue;
                const nf = f + df;
                const nr = r + dr;
                if (nf >= 0 and nf < 8 and nr >= 0 and nr < 8) {
                    const nsq = Square.make(@enumFromInt(@as(u3, @intCast(nf))), @enumFromInt(@as(u3, @intCast(nr))));
                    bb.set(nsq);
                }
            }
        }
        tbl[i] = bb;
    }
    break :blk tbl;
};

/// pawnAttacks[color][square] = bitboard of squares a pawn on `square` attacks.
/// color is attacker color (white pawns attack rank+1).
pub const pawnAttacks: [2][64]Bitboard = blk: {
    @setEvalBranchQuota(10000);
    var tbl: [2][64]Bitboard = undefined;
    for (0..2) |c| {
        for (0..64) |i| {
            const sq: Square = @enumFromInt(i);
            const f: i8 = @intFromEnum(sq.file());
            const r: i8 = @intFromEnum(sq.rank());
            var bb = Bitboard.empty;
            const dr: i8 = if (c == 0) 1 else -1; // white=0, black=1 matches Color enum
            const nr = r + dr;
            if (nr >= 0 and nr < 8) {
                if (f - 1 >= 0) {
                    const nsq = Square.make(@enumFromInt(@as(u3, @intCast(f - 1))), @enumFromInt(@as(u3, @intCast(nr))));
                    bb.set(nsq);
                }
                if (f + 1 < 8) {
                    const nsq = Square.make(@enumFromInt(@as(u3, @intCast(f + 1))), @enumFromInt(@as(u3, @intCast(nr))));
                    bb.set(nsq);
                }
            }
            tbl[c][i] = bb;
        }
    }
    break :blk tbl;
};

pub inline fn knightAttacksAt(sq: Square) Bitboard {
    return knightAttacks[@intFromEnum(sq)];
}
pub inline fn kingAttacksAt(sq: Square) Bitboard {
    return kingAttacks[@intFromEnum(sq)];
}
pub inline fn pawnAttacksAt(sq: Square, color: Color) Bitboard {
    return pawnAttacks[@intFromEnum(color)][@intFromEnum(sq)];
}

// ── Sliding attacks (plain ray loops, blocked by occupancy) ────────────

const BishopDirs = [_][2]i8{ .{ 1, 1 }, .{ 1, -1 }, .{ -1, 1 }, .{ -1, -1 } };
const RookDirs = [_][2]i8{ .{ 1, 0 }, .{ -1, 0 }, .{ 0, 1 }, .{ 0, -1 } };

pub fn bishopAttacks(sq: Square, occupied: Bitboard) Bitboard {
    return slidingAttacks(sq, occupied, &BishopDirs);
}

pub fn rookAttacks(sq: Square, occupied: Bitboard) Bitboard {
    return slidingAttacks(sq, occupied, &RookDirs);
}

pub fn queenAttacks(sq: Square, occupied: Bitboard) Bitboard {
    return Bitboard.fromRaw(bishopAttacks(sq, occupied).bits | rookAttacks(sq, occupied).bits);
}

fn slidingAttacks(sq: Square, occupied: Bitboard, dirs: []const [2]i8) Bitboard {
    var result = Bitboard.empty;
    const f0: i8 = @intFromEnum(sq.file());
    const r0: i8 = @intFromEnum(sq.rank());
    for (dirs) |d| {
        var f = f0 + d[0];
        var r = r0 + d[1];
        while (f >= 0 and f < 8 and r >= 0 and r < 8) {
            const nsq = Square.make(@enumFromInt(@as(u3, @intCast(f))), @enumFromInt(@as(u3, @intCast(r))));
            result.set(nsq);
            if (occupied.contains(nsq)) break; // blocked
            f += d[0];
            r += d[1];
        }
    }
    return result;
}

/// Squares between (exclusive) — for pin / block checks (optional utility)
pub fn squaresBetween(a: Square, b: Square) Bitboard {
    // Only meaningful if aligned; otherwise empty.
    // Generate by direction from a toward b.
    const af: i8 = @intFromEnum(a.file());
    const ar: i8 = @intFromEnum(a.rank());
    const bf: i8 = @intFromEnum(b.file());
    const br: i8 = @intFromEnum(b.rank());
    const df = bf - af;
    const dr = br - ar;
    const step_f: i8 = if (df == 0) 0 else if (df > 0) 1 else -1;
    const step_r: i8 = if (dr == 0) 0 else if (dr > 0) 1 else -1;
    // check alignment: straight or diagonal
    if (step_f != 0 and step_r != 0 and @abs(df) != @abs(dr)) return Bitboard.empty;
    if (step_f == 0 and step_r == 0) return Bitboard.empty;
    var bb = Bitboard.empty;
    var f = af + step_f;
    var r = ar + step_r;
    while (f != bf or r != br) {
        const nsq = Square.make(@enumFromInt(@as(u3, @intCast(f))), @enumFromInt(@as(u3, @intCast(r))));
        if (nsq == b) break;
        bb.set(nsq);
        f += step_f;
        r += step_r;
    }
    return bb;
}

// ── Attack detection ───────────────────────────────────────────────────

/// Is `sq` attacked by `attacker` color? Uses board occupancies and piece bitboards.
pub fn isSquareAttacked(board: Board, sq: Square, attacker: Color) bool {
    // Pawn attacks: reverse lookup — pawns that could attack sq are on rank +/-1
    // Instead of checking all pawns, check pawn attack table from sq's perspective:
    // square `sq` is attacked by pawn if attacker pawn is on a square that attacks `sq`.
    // That's equivalent to: attacker pawn bitboard intersects reverse pawn attacks.
    const pawn_bb = board.pieces[@intFromEnum(if (attacker == .white) piece_mod.Piece.white_pawn else piece_mod.Piece.black_pawn)];
    // Pawns attack diagonally forward; to find attackers of sq, look one rank opposite.
    // White attacker: pawns are one rank below sq (since they attack upward). So check squares r-1.
    // We can compute by checking pawnAttacks of potential attacker squares, but easier to test via table:
    // Generate pawn attack origins: for white, attackers are on rank sq.rank()-1, file +/-1.
    {
        const f: i8 = @intFromEnum(sq.file());
        const r: i8 = @intFromEnum(sq.rank());
        var attackers = Bitboard.empty;
        if (attacker == .white) {
            const pr = r - 1;
            if (pr >= 0) {
                if (f - 1 >= 0) attackers.set(Square.make(@enumFromInt(@as(u3, @intCast(f - 1))), @enumFromInt(@as(u3, @intCast(pr)))));
                if (f + 1 < 8) attackers.set(Square.make(@enumFromInt(@as(u3, @intCast(f + 1))), @enumFromInt(@as(u3, @intCast(pr)))));
            }
        } else {
            const pr = r + 1;
            if (pr < 8) {
                if (f - 1 >= 0) attackers.set(Square.make(@enumFromInt(@as(u3, @intCast(f - 1))), @enumFromInt(@as(u3, @intCast(pr)))));
                if (f + 1 < 8) attackers.set(Square.make(@enumFromInt(@as(u3, @intCast(f + 1))), @enumFromInt(@as(u3, @intCast(pr)))));
            }
        }
        if (pawn_bb.intersectWith(attackers).isNotEmpty()) return true;
    }

    // Knight
    const knights = board.pieces[@intFromEnum(if (attacker == .white) piece_mod.Piece.white_knight else piece_mod.Piece.black_knight)];
    if (knights.intersectWith(knightAttacksAt(sq)).isNotEmpty()) return true;

    // King
    const kings = board.pieces[@intFromEnum(if (attacker == .white) piece_mod.Piece.white_king else piece_mod.Piece.black_king)];
    if (kings.intersectWith(kingAttacksAt(sq)).isNotEmpty()) return true;

    const occupied = board.occupied_all;

    // Bishop / queen diagonal
    const bishop_queens = blk: {
        const b1 = board.pieces[@intFromEnum(if (attacker == .white) piece_mod.Piece.white_bishop else piece_mod.Piece.black_bishop)];
        const b2 = board.pieces[@intFromEnum(if (attacker == .white) piece_mod.Piece.white_queen else piece_mod.Piece.black_queen)];
        break :blk Bitboard.fromRaw(b1.bits | b2.bits);
    };
    if (bishop_queens.isNotEmpty()) {
        const attacks = bishopAttacks(sq, occupied);
        if (attacks.intersectWith(bishop_queens).isNotEmpty()) return true;
    }

    // Rook / queen orthogonal
    const rook_queens = blk: {
        const b1 = board.pieces[@intFromEnum(if (attacker == .white) piece_mod.Piece.white_rook else piece_mod.Piece.black_rook)];
        const b2 = board.pieces[@intFromEnum(if (attacker == .white) piece_mod.Piece.white_queen else piece_mod.Piece.black_queen)];
        break :blk Bitboard.fromRaw(b1.bits | b2.bits);
    };
    if (rook_queens.isNotEmpty()) {
        const attacks = rookAttacks(sq, occupied);
        if (attacks.intersectWith(rook_queens).isNotEmpty()) return true;
    }

    return false;
}

// ── Tests ────────────────────────────────────────────────────────────────

test "knight attacks" {
    try std.testing.expectEqual(@as(u7, 8), knightAttacksAt(.e4).count());
    try std.testing.expectEqual(@as(u7, 2), knightAttacksAt(.a1).count());
    try std.testing.expect(knightAttacksAt(.e4).contains(.d6));
    try std.testing.expect(!knightAttacksAt(.e4).contains(.e5));
}

test "king attacks" {
    try std.testing.expectEqual(@as(u7, 3), kingAttacksAt(.a1).count());
    try std.testing.expectEqual(@as(u7, 8), kingAttacksAt(.e4).count());
    try std.testing.expectEqual(@as(u7, 5), kingAttacksAt(.a4).count());
}

test "pawn attacks" {
    // white pawn on e4 attacks d5 and f5
    const w = pawnAttacksAt(.e4, .white);
    try std.testing.expect(w.contains(.d5));
    try std.testing.expect(w.contains(.f5));
    try std.testing.expectEqual(@as(u7, 2), w.count());
    // black pawn on e4 attacks d3 and f3
    const b = pawnAttacksAt(.e4, .black);
    try std.testing.expect(b.contains(.d3));
    try std.testing.expect(b.contains(.f3));
    // edge
    try std.testing.expectEqual(@as(u7, 1), pawnAttacksAt(.a2, .white).count());
}

test "sliding attacks empty board" {
    const occupied = Bitboard.empty;
    try std.testing.expectEqual(@as(u7, 13), bishopAttacks(.d4, occupied).count());
    try std.testing.expectEqual(@as(u7, 14), rookAttacks(.d4, occupied).count());
    try std.testing.expectEqual(@as(u7, 27), queenAttacks(.d4, occupied).count());
    try std.testing.expectEqual(@as(u7, 7), bishopAttacks(.a1, occupied).count());
}

test "sliding attacks blocked" {
    var occ = Bitboard.empty;
    occ.set(.d6); // block north of d4
    occ.set(.f4); // block east
    const r = rookAttacks(.d4, occ);
    try std.testing.expect(r.contains(.d5));
    try std.testing.expect(r.contains(.d6));
    try std.testing.expect(!r.contains(.d7)); // blocked
    try std.testing.expect(r.contains(.e4));
    try std.testing.expect(r.contains(.f4));
    try std.testing.expect(!r.contains(.g4));
}

test "isSquareAttacked startpos" {
    const board = board_mod.Board.startingPosition();
    // e1 white king not attacked by black in startpos
    try std.testing.expect(!isSquareAttacked(board, .e1, .black));
    try std.testing.expect(!isSquareAttacked(board, .e8, .white));
    // knight attacks: b1 white knight attacks a3 and c3 — is a3 attacked by white?
    try std.testing.expect(isSquareAttacked(board, .a3, .white));
    try std.testing.expect(isSquareAttacked(board, .c3, .white));
    // pawn attacks: white pawn on e2 attacks d3 and f3
    // Actually startpos white pawns not yet attacking? They do attack d3/f3 even though pawns on e2
    try std.testing.expect(isSquareAttacked(board, .d3, .white));
}

test "isSquareAttacked custom" {
    var b = Board.empty();
    b.setPiece(.e4, .white_queen);
    b.setPiece(.e1, .white_king);
    b.setPiece(.e8, .black_king);
    // queen on e4 attacks e8 along file if no blocker
    try std.testing.expect(isSquareAttacked(b, .e8, .white));
    b.setPiece(.e6, .black_pawn); // block
    try std.testing.expect(!isSquareAttacked(b, .e8, .white)); // now blocked by pawn on e6? queen ray blocked
    // but pawn itself maybe attacks differently — not relevant
}

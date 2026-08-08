const std = @import("std");

pub const UciCommand = union(enum) {
    uci,
    isready,
    ucinewgame,
    quit,
    stop,
    position_startpos: struct { moves: []const []const u8 }, // moves are slices into line buffer
    position_fen: struct { fen: []const u8, moves: []const []const u8 },
    go_depth: u8,
    go_movetime: u32, // ms
    go_infinite,
    unknown: []const u8,
};

// Fixed-buffer parser: line is 0..8191 bytes, no allocation beyond arena for moves.
// Caller provides buffer for line (already trimmed). We parse into arena for moves slices.
pub fn parseLine(line: []const u8, arena: std.mem.Allocator) !UciCommand {
    const trimmed = std.mem.trim(u8, line, &std.ascii.whitespace);
    if (trimmed.len == 0) return error.EmptyLine;
    if (std.mem.eql(u8, trimmed, "uci")) return .uci;
    if (std.mem.eql(u8, trimmed, "isready")) return .isready;
    if (std.mem.eql(u8, trimmed, "ucinewgame")) return .ucinewgame;
    if (std.mem.eql(u8, trimmed, "quit") or std.mem.eql(u8, trimmed, "exit")) return .quit;
    if (std.mem.eql(u8, trimmed, "stop")) return .stop;

    if (std.mem.startsWith(u8, trimmed, "position ")) {
        const rest = trimmed["position ".len..];
        if (std.mem.startsWith(u8, rest, "startpos")) {
            var moves: []const []const u8 = &.{};
            const moves_idx = std.mem.indexOf(u8, rest, "moves");
            if (moves_idx) |idx| {
                const moves_str = std.mem.trim(u8, rest[idx + "moves".len ..], &std.ascii.whitespace);
                if (moves_str.len > 0) {
                    var list: std.ArrayList([]const u8) = .empty;
                    var it = std.mem.tokenizeScalar(u8, moves_str, ' ');
                    while (it.next()) |m| try list.append(arena, m);
                    moves = try list.toOwnedSlice(arena);
                }
            }
            return .{ .position_startpos = .{ .moves = moves } };
        } else if (std.mem.startsWith(u8, rest, "fen ")) {
            const fen_start = "fen ".len;
            const moves_idx = std.mem.indexOf(u8, rest, " moves ");
            if (moves_idx) |idx| {
                const fen_part = std.mem.trim(u8, rest[fen_start..idx], &std.ascii.whitespace);
                const moves_str = std.mem.trim(u8, rest[idx + " moves ".len ..], &std.ascii.whitespace);
                var moves: []const []const u8 = &.{};
                if (moves_str.len > 0) {
                    var list: std.ArrayList([]const u8) = .empty;
                    var it = std.mem.tokenizeScalar(u8, moves_str, ' ');
                    while (it.next()) |m| try list.append(arena, m);
                    moves = try list.toOwnedSlice(arena);
                }
                const fen_copy = try arena.dupe(u8, fen_part);
                return .{ .position_fen = .{ .fen = fen_copy, .moves = moves } };
            } else {
                const fen_part = std.mem.trim(u8, rest[fen_start..], &std.ascii.whitespace);
                const fen_copy = try arena.dupe(u8, fen_part);
                return .{ .position_fen = .{ .fen = fen_copy, .moves = &.{}} };
            }
        }
    }

    if (std.mem.startsWith(u8, trimmed, "go ")) {
        const rest = trimmed["go ".len..];
        if (std.mem.eql(u8, rest, "infinite")) return .go_infinite;
        var it = std.mem.tokenizeScalar(u8, rest, ' ');
        while (it.next()) |tok| {
            if (std.mem.eql(u8, tok, "depth")) {
                const v = it.next() orelse return .{ .unknown = trimmed };
                const d = std.fmt.parseInt(u8, v, 10) catch return .{ .unknown = trimmed };
                return .{ .go_depth = d };
            } else if (std.mem.eql(u8, tok, "movetime")) {
                const v = it.next() orelse return .{ .unknown = trimmed };
                const ms = std.fmt.parseInt(u32, v, 10) catch return .{ .unknown = trimmed };
                return .{ .go_movetime = ms };
            } else if (std.mem.eql(u8, tok, "wtime") or std.mem.eql(u8, tok, "btime") or std.mem.eql(u8, tok, "winc") or std.mem.eql(u8, tok, "binc") or std.mem.eql(u8, tok, "movestogo")) {
                _ = it.next(); // skip value
                continue;
            }
        }
        // If go with no recognized subcommand, treat as infinite for now
        return .go_infinite;
    }

    return .{ .unknown = trimmed };
}

test "uci parse" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    try std.testing.expectEqual(UciCommand.uci, try parseLine("uci", alloc));
    try std.testing.expectEqual(UciCommand.isready, try parseLine("isready", alloc));
    const p1 = try parseLine("position startpos", alloc);
    try std.testing.expect(p1 == .position_startpos);
    const p2 = try parseLine("position startpos moves e2e4 e7e5", alloc);
    try std.testing.expectEqual(@as(usize, 2), p2.position_startpos.moves.len);
    const g1 = try parseLine("go depth 4", alloc);
    try std.testing.expectEqual(@as(u8, 4), g1.go_depth);
    const g2 = try parseLine("go movetime 1000", alloc);
    try std.testing.expectEqual(@as(u32, 1000), g2.go_movetime);
}

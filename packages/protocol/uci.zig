const std = @import("std");

pub const UciCommand = union(enum) {
    uci,
    debug: bool, // true = on, false = off
    isready,
    setoption: struct { name: []const u8, value: ?[]const u8 },
    register: struct { later: bool, name: ?[]const u8, code: ?[]const u8 },
    ucinewgame,
    quit,
    stop,
    ponderhit,
    position_startpos: struct { moves: []const []const u8 },
    position_fen: struct { fen: []const u8, moves: []const []const u8 },
    go: GoParams,
    perft: struct { depth: u32, divide: bool },
    bench: struct { depth: ?u8 },
    display, // `d`
    eval,
    compiler,
    speedtest: struct { threads: ?u8, hash: ?u32, secs: ?u32 },
    unknown: []const u8,
};

pub const GoParams = struct {
    searchmoves: ?[]const []const u8 = null,
    ponder: bool = false,
    wtime: ?u32 = null,
    btime: ?u32 = null,
    winc: ?u32 = null,
    binc: ?u32 = null,
    movestogo: ?u32 = null,
    depth: ?u8 = null,
    nodes: ?u64 = null,
    mate: ?u8 = null,
    movetime: ?u32 = null,
    infinite: bool = false,
    // Legacy helpers for old call sites (still valid)
    pub fn isDepth(self: GoParams) ?u8 { return self.depth; }
    pub fn isInfinite(self: GoParams) bool { return self.infinite; }
};

// Fixed-buffer parser: line is 0..8191 bytes, no allocation beyond arena for moves.
pub fn parseLine(line: []const u8, arena: std.mem.Allocator) !UciCommand {
    const trimmed = std.mem.trim(u8, line, &std.ascii.whitespace);
    if (trimmed.len == 0) return error.EmptyLine;
    if (std.mem.eql(u8, trimmed, "uci")) return .uci;
    if (std.mem.eql(u8, trimmed, "isready")) return .isready;
    if (std.mem.eql(u8, trimmed, "ucinewgame")) return .ucinewgame;
    if (std.mem.eql(u8, trimmed, "quit") or std.mem.eql(u8, trimmed, "exit")) return .quit;
    if (std.mem.eql(u8, trimmed, "stop")) return .stop;
    if (std.mem.eql(u8, trimmed, "ponderhit")) return .ponderhit;
    if (std.mem.eql(u8, trimmed, "debug on")) return .{ .debug = true };
    if (std.mem.eql(u8, trimmed, "debug off")) return .{ .debug = false };
    if (std.mem.startsWith(u8, trimmed, "debug ")) {
        const rest = std.mem.trim(u8, trimmed["debug ".len..], &std.ascii.whitespace);
        if (std.mem.eql(u8, rest, "on")) return .{ .debug = true };
        if (std.mem.eql(u8, rest, "off")) return .{ .debug = false };
        return .{ .unknown = trimmed };
    }
    if (std.mem.startsWith(u8, trimmed, "setoption ")) {
        // setoption name <id> [value <x>]
        const rest = trimmed["setoption ".len..];
        const name_idx = std.mem.indexOf(u8, rest, "name ");
        if (name_idx == null) return .{ .unknown = trimmed };
        const after_name = rest[name_idx.? + "name ".len ..];
        // Handle both " value " and trailing " value" (empty value, e.g. SyzygyPath cleared)
        const value_idx = std.mem.indexOf(u8, after_name, " value ");
        const value_idx2 = std.mem.indexOf(u8, after_name, " value");
        if (value_idx) |vi| {
            const name_part = std.mem.trim(u8, after_name[0..vi], &std.ascii.whitespace);
            const value_part = std.mem.trim(u8, after_name[vi + " value ".len ..], &std.ascii.whitespace);
            const name_copy = try arena.dupe(u8, name_part);
            const value_copy = try arena.dupe(u8, value_part);
            return .{ .setoption = .{ .name = name_copy, .value = value_copy } };
        } else if (value_idx2) |vi| {
            // " value" at end (maybe empty or " value" without trailing space)
            // Ensure it's at end or followed by nothing after trimming
            const after_val = after_name[vi + " value".len ..];
            const trimmed_after = std.mem.trim(u8, after_val, &std.ascii.whitespace);
            if (trimmed_after.len == 0) {
                const name_part = std.mem.trim(u8, after_name[0..vi], &std.ascii.whitespace);
                const name_copy = try arena.dupe(u8, name_part);
                // Empty value means clear (e.g. SyzygyPath)
                const value_copy = try arena.dupe(u8, "");
                return .{ .setoption = .{ .name = name_copy, .value = value_copy } };
            }
            const name_part = std.mem.trim(u8, after_name[0..vi], &std.ascii.whitespace);
            const value_part = std.mem.trim(u8, after_val, &std.ascii.whitespace);
            const name_copy = try arena.dupe(u8, name_part);
            const value_copy = try arena.dupe(u8, value_part);
            return .{ .setoption = .{ .name = name_copy, .value = value_copy } };
        } else {
            const name_part = std.mem.trim(u8, after_name, &std.ascii.whitespace);
            const name_copy = try arena.dupe(u8, name_part);
            return .{ .setoption = .{ .name = name_copy, .value = null } };
        }
    }
    if (std.mem.startsWith(u8, trimmed, "register")) {
        const rest = std.mem.trim(u8, trimmed["register".len..], &std.ascii.whitespace);
        if (rest.len == 0) return .{ .register = .{ .later = false, .name = null, .code = null } };
        if (std.mem.eql(u8, rest, "later")) return .{ .register = .{ .later = true, .name = null, .code = null } };
        // register name <x> [code <y>] or code <y>
        var later = false;
        var name: ?[]const u8 = null;
        var code: ?[]const u8 = null;
        var it = std.mem.tokenizeScalar(u8, rest, ' ');
        while (it.next()) |tok| {
            if (std.mem.eql(u8, tok, "later")) later = true
            else if (std.mem.eql(u8, tok, "name")) {
                if (it.next()) |v| name = try arena.dupe(u8, v);
            } else if (std.mem.eql(u8, tok, "code")) {
                if (it.next()) |v| code = try arena.dupe(u8, v);
            }
        }
        return .{ .register = .{ .later = later, .name = name, .code = code } };
    }

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

    // --- Custom debug/test commands (non-standard, separate from UCI) ---
    if (std.mem.eql(u8, trimmed, "d")) return .display;
    if (std.mem.eql(u8, trimmed, "eval") or std.mem.eql(u8, trimmed, "e")) return .eval;
    if (std.mem.eql(u8, trimmed, "compiler")) return .compiler;
    if (std.mem.startsWith(u8, trimmed, "perft")) {
        const rest = std.mem.trim(u8, if (trimmed.len > 5) trimmed[5..] else "", &std.ascii.whitespace);
        if (rest.len == 0) return .{ .perft = .{ .depth = 0, .divide = false } }; // missing arg -> handled as error by caller
        var it = std.mem.tokenizeScalar(u8, rest, ' ');
        const tok = it.next() orelse return .{ .perft = .{ .depth = 0, .divide = false } };
        const d = std.fmt.parseInt(u32, tok, 10) catch 999;
        var divide = false;
        while (it.next()) |t| {
            if (std.mem.eql(u8, t, "divide")) divide = true;
        }
        return .{ .perft = .{ .depth = d, .divide = divide } };
    }
    if (std.mem.eql(u8, trimmed, "bench")) return .{ .bench = .{ .depth = null } };
    if (std.mem.startsWith(u8, trimmed, "bench ")) {
        const rest = std.mem.trim(u8, trimmed["bench ".len..], &std.ascii.whitespace);
        const d = std.fmt.parseInt(u8, rest, 10) catch 255;
        return .{ .bench = .{ .depth = d } };
    }
    if (std.mem.eql(u8, trimmed, "speedtest")) return .{ .speedtest = .{ .threads = null, .hash = null, .secs = null } };
    if (std.mem.startsWith(u8, trimmed, "speedtest ")) {
        const rest = std.mem.trim(u8, trimmed["speedtest ".len..], &std.ascii.whitespace);
        var it = std.mem.tokenizeScalar(u8, rest, ' ');
        var thr: ?u8 = null;
        var h: ?u32 = null;
        var s: ?u32 = null;
        var has_invalid = false;
        if (it.next()) |t| {
            thr = std.fmt.parseInt(u8, t, 10) catch null;
            if (thr == null) {
                has_invalid = true;
                thr = 255; // sentinel triggers error in caller
            }
        }
        if (it.next()) |t| {
            const parsed = std.fmt.parseInt(u32, t, 10) catch null;
            if (parsed == null) {
                has_invalid = true;
                h = 9999;
            } else h = parsed;
        }
        if (it.next()) |t| {
            const parsed = std.fmt.parseInt(u32, t, 10) catch null;
            if (parsed == null) {
                has_invalid = true;
                s = 9999;
            } else s = parsed;
        }
        // If we saw tokens but all failed, ensure at least one invalid sentinel so caller reports error not default
        if (has_invalid) {
            if (thr == null) thr = 255;
            if (h == null) h = 9999;
            if (s == null) s = 9999;
        }
        return .{ .speedtest = .{ .threads = thr, .hash = h, .secs = s } };
    }

    if (std.mem.startsWith(u8, trimmed, "go")) {
        const rest = if (trimmed.len > 2) std.mem.trim(u8, trimmed["go".len..], &std.ascii.whitespace) else "";
        if (rest.len == 0) return .{ .go = .{} };
        var params = GoParams{};
        var it = std.mem.tokenizeScalar(u8, rest, ' ');
        var searchmoves_list: std.ArrayList([]const u8) = .empty;
        var in_searchmoves = false;
        while (it.next()) |tok| {
            if (in_searchmoves) {
                // searchmoves list continues until next known token
                if (std.mem.eql(u8, tok, "ponder") or std.mem.eql(u8, tok, "wtime") or std.mem.eql(u8, tok, "btime") or std.mem.eql(u8, tok, "winc") or std.mem.eql(u8, tok, "binc") or std.mem.eql(u8, tok, "movestogo") or std.mem.eql(u8, tok, "depth") or std.mem.eql(u8, tok, "nodes") or std.mem.eql(u8, tok, "mate") or std.mem.eql(u8, tok, "movetime") or std.mem.eql(u8, tok, "infinite")) {
                    in_searchmoves = false;
                    // Re-process this token as non-searchmoves
                    if (std.mem.eql(u8, tok, "ponder")) params.ponder = true
                    else if (std.mem.eql(u8, tok, "wtime")) { params.wtime = std.fmt.parseInt(u32, it.next() orelse "0", 10) catch null; }
                    else if (std.mem.eql(u8, tok, "btime")) { params.btime = std.fmt.parseInt(u32, it.next() orelse "0", 10) catch null; }
                    else if (std.mem.eql(u8, tok, "winc")) { params.winc = std.fmt.parseInt(u32, it.next() orelse "0", 10) catch null; }
                    else if (std.mem.eql(u8, tok, "binc")) { params.binc = std.fmt.parseInt(u32, it.next() orelse "0", 10) catch null; }
                    else if (std.mem.eql(u8, tok, "movestogo")) { params.movestogo = std.fmt.parseInt(u32, it.next() orelse "0", 10) catch null; }
                    else if (std.mem.eql(u8, tok, "depth")) { params.depth = std.fmt.parseInt(u8, it.next() orelse "0", 10) catch null; }
                    else if (std.mem.eql(u8, tok, "nodes")) { params.nodes = std.fmt.parseInt(u64, it.next() orelse "0", 10) catch null; }
                    else if (std.mem.eql(u8, tok, "mate")) { params.mate = std.fmt.parseInt(u8, it.next() orelse "0", 10) catch null; }
                    else if (std.mem.eql(u8, tok, "movetime")) { params.movetime = std.fmt.parseInt(u32, it.next() orelse "0", 10) catch null; }
                    else if (std.mem.eql(u8, tok, "infinite")) params.infinite = true;
                    continue;
                } else {
                    try searchmoves_list.append(arena, tok);
                    continue;
                }
            }
            if (std.mem.eql(u8, tok, "searchmoves")) {
                in_searchmoves = true;
                continue;
            } else if (std.mem.eql(u8, tok, "ponder")) {
                params.ponder = true;
            } else if (std.mem.eql(u8, tok, "infinite")) {
                params.infinite = true;
            } else if (std.mem.eql(u8, tok, "wtime")) {
                const v = it.next() orelse continue;
                params.wtime = std.fmt.parseInt(u32, v, 10) catch null;
            } else if (std.mem.eql(u8, tok, "btime")) {
                const v = it.next() orelse continue;
                params.btime = std.fmt.parseInt(u32, v, 10) catch null;
            } else if (std.mem.eql(u8, tok, "winc")) {
                const v = it.next() orelse continue;
                params.winc = std.fmt.parseInt(u32, v, 10) catch null;
            } else if (std.mem.eql(u8, tok, "binc")) {
                const v = it.next() orelse continue;
                params.binc = std.fmt.parseInt(u32, v, 10) catch null;
            } else if (std.mem.eql(u8, tok, "movestogo")) {
                const v = it.next() orelse continue;
                params.movestogo = std.fmt.parseInt(u32, v, 10) catch null;
            } else if (std.mem.eql(u8, tok, "depth")) {
                const v = it.next() orelse continue;
                params.depth = std.fmt.parseInt(u8, v, 10) catch null;
            } else if (std.mem.eql(u8, tok, "nodes")) {
                const v = it.next() orelse continue;
                params.nodes = std.fmt.parseInt(u64, v, 10) catch null;
            } else if (std.mem.eql(u8, tok, "mate")) {
                const v = it.next() orelse continue;
                params.mate = std.fmt.parseInt(u8, v, 10) catch null;
            } else if (std.mem.eql(u8, tok, "movetime")) {
                const v = it.next() orelse continue;
                params.movetime = std.fmt.parseInt(u32, v, 10) catch null;
            } else {
                // Unknown go subcommand — treat as searchmoves if not recognized? Skip.
                continue;
            }
        }
        if (searchmoves_list.items.len > 0) {
            params.searchmoves = try searchmoves_list.toOwnedSlice(arena);
        }
        // Normalize: if no limit specified, treat as infinite (or depth 3 fallback later)
        return .{ .go = params };
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
    try std.testing.expectEqual(@as(u8, 4), g1.go.depth.?);
    const g2 = try parseLine("go movetime 1000", alloc);
    try std.testing.expectEqual(@as(u32, 1000), g2.go.movetime.?);
}

test "uci parse full go" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const g = try parseLine("go wtime 1000 btime 1000 winc 10 binc 10 movestogo 40 depth 6 nodes 1000 movetime 500 infinite ponder searchmoves e2e4 d2d4", alloc);
    try std.testing.expectEqual(@as(u32, 1000), g.go.wtime.?);
    try std.testing.expectEqual(@as(u32, 500), g.go.movetime.?);
    try std.testing.expect(g.go.infinite);
    try std.testing.expect(g.go.ponder);
    try std.testing.expectEqual(@as(usize, 2), g.go.searchmoves.?.len);
}

test "uci parse setoption" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const s1 = try parseLine("setoption name Threads value 4", alloc);
    try std.testing.expectEqualStrings("Threads", s1.setoption.name);
    try std.testing.expectEqualStrings("4", s1.setoption.value.?);
    const s2 = try parseLine("setoption name Hash value 128", alloc);
    try std.testing.expectEqualStrings("Hash", s2.setoption.name);
}

test "uci parse debug register ponderhit" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    try std.testing.expectEqual(true, (try parseLine("debug on", alloc)).debug);
    try std.testing.expectEqual(false, (try parseLine("debug off", alloc)).debug);
    try std.testing.expect((try parseLine("register later", alloc)).register.later);
    try std.testing.expectEqual(UciCommand.ponderhit, try parseLine("ponderhit", alloc));
}

test "uci parse debug commands" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    try std.testing.expect((try parseLine("d", alloc)) == .display);
    try std.testing.expect((try parseLine("eval", alloc)) == .eval);
    try std.testing.expect((try parseLine("compiler", alloc)) == .compiler);
    try std.testing.expect((try parseLine("bench", alloc)).bench.depth == null);
    try std.testing.expectEqual(@as(u8, 4), (try parseLine("bench 4", alloc)).bench.depth.?);
    try std.testing.expectEqual(@as(u32, 3), (try parseLine("perft 3", alloc)).perft.depth);
    try std.testing.expect((try parseLine("perft 3 divide", alloc)).perft.divide);
    try std.testing.expectEqual(@as(u8, 2), (try parseLine("speedtest 2 64 5", alloc)).speedtest.threads.?);
}

test "uci parse perft invalid" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    try std.testing.expectEqual(@as(u32, 999), (try parseLine("perft bad", alloc)).perft.depth);
}

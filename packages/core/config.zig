const std = @import("std");

pub const OptionError = error{
    UnknownOption,
    MissingValue,
    InvalidValue,
};

pub const OptionType = enum {
    spin,
    check,
    string,
    button,
};

pub const OptionDecl = struct {
    name: []const u8,
    opt_type: OptionType,
    default: []const u8,
    min: ?i64 = null,
    max: ?i64 = null,
};

pub const option_decls: []const OptionDecl = &.{
    .{ .name = "Threads", .opt_type = .spin, .default = "1", .min = 1, .max = 16 },
    .{ .name = "Hash", .opt_type = .spin, .default = "16", .min = 1, .max = 1024 },
    .{ .name = "MoveOverhead", .opt_type = .spin, .default = "30", .min = 0, .max = 5000 },
    .{ .name = "OwnBook", .opt_type = .check, .default = "true" },
    .{ .name = "Clear Hash", .opt_type = .button, .default = "" },
    .{ .name = "SyzygyPath", .opt_type = .string, .default = "<empty>" },
    .{ .name = "SyzygyProbeDepth", .opt_type = .spin, .default = "1", .min = 1, .max = 100 },
    .{ .name = "SyzygyProbeLimit", .opt_type = .spin, .default = "7", .min = 0, .max = 7 },
    .{ .name = "Syzygy50MoveRule", .opt_type = .check, .default = "true" },
};

pub const EngineConfig = struct {
    use_opening_book: bool = true,
    hash_mb: u32 = 16,
    threads: u8 = 1,
    move_overhead_ms: u32 = 30,
    syzygy_path: []const u8 = "",
    syzygy_probe_depth: u8 = 1,
    syzygy_probe_limit: u8 = 7,
    syzygy_50_move_rule: bool = true,

    pub fn setOption(self: *EngineConfig, name: []const u8, value: ?[]const u8) OptionError!void {
        if (std.mem.eql(u8, name, "Threads")) {
            const v = value orelse return OptionError.MissingValue;
            const trimmed = std.mem.trim(u8, v, &std.ascii.whitespace);
            if (trimmed.len == 0) return OptionError.MissingValue;
            const parsed = std.fmt.parseInt(i64, trimmed, 10) catch return OptionError.InvalidValue;
            if (parsed < 1 or parsed > 16) return OptionError.InvalidValue;
            self.threads = @intCast(parsed);
            return;
        } else if (std.mem.eql(u8, name, "Hash")) {
            const v = value orelse return OptionError.MissingValue;
            const trimmed = std.mem.trim(u8, v, &std.ascii.whitespace);
            if (trimmed.len == 0) return OptionError.MissingValue;
            const parsed = std.fmt.parseInt(i64, trimmed, 10) catch return OptionError.InvalidValue;
            if (parsed < 1 or parsed > 1024) return OptionError.InvalidValue;
            self.hash_mb = @intCast(parsed);
            return;
        } else if (std.mem.eql(u8, name, "MoveOverhead")) {
            const v = value orelse return OptionError.MissingValue;
            const trimmed = std.mem.trim(u8, v, &std.ascii.whitespace);
            if (trimmed.len == 0) return OptionError.MissingValue;
            const parsed = std.fmt.parseInt(i64, trimmed, 10) catch return OptionError.InvalidValue;
            if (parsed < 0 or parsed > 5000) return OptionError.InvalidValue;
            self.move_overhead_ms = @intCast(parsed);
            return;
        } else if (std.mem.eql(u8, name, "OwnBook")) {
            const v = value orelse return OptionError.MissingValue;
            const trimmed = std.mem.trim(u8, v, &std.ascii.whitespace);
            if (trimmed.len == 0) return OptionError.MissingValue;
            if (parseBool(trimmed)) |b| {
                self.use_opening_book = b;
                return;
            } else return OptionError.InvalidValue;
        } else if (std.mem.eql(u8, name, "Clear Hash")) {
            // button: value ignored
            return;
        } else if (std.mem.eql(u8, name, "SyzygyPath")) {
            // string: accept empty, missing value means clear
            if (value) |v| {
                self.syzygy_path = v;
            } else {
                self.syzygy_path = "";
            }
            return;
        } else if (std.mem.eql(u8, name, "SyzygyProbeDepth")) {
            const v = value orelse return OptionError.MissingValue;
            const trimmed = std.mem.trim(u8, v, &std.ascii.whitespace);
            if (trimmed.len == 0) return OptionError.MissingValue;
            const parsed = std.fmt.parseInt(i64, trimmed, 10) catch return OptionError.InvalidValue;
            if (parsed < 1 or parsed > 100) return OptionError.InvalidValue;
            self.syzygy_probe_depth = @intCast(parsed);
            return;
        } else if (std.mem.eql(u8, name, "SyzygyProbeLimit")) {
            const v = value orelse return OptionError.MissingValue;
            const trimmed = std.mem.trim(u8, v, &std.ascii.whitespace);
            if (trimmed.len == 0) return OptionError.MissingValue;
            const parsed = std.fmt.parseInt(i64, trimmed, 10) catch return OptionError.InvalidValue;
            if (parsed < 0 or parsed > 7) return OptionError.InvalidValue;
            self.syzygy_probe_limit = @intCast(parsed);
            return;
        } else if (std.mem.eql(u8, name, "Syzygy50MoveRule")) {
            const v = value orelse return OptionError.MissingValue;
            const trimmed = std.mem.trim(u8, v, &std.ascii.whitespace);
            if (trimmed.len == 0) return OptionError.MissingValue;
            if (parseBool(trimmed)) |b| {
                self.syzygy_50_move_rule = b;
                return;
            } else return OptionError.InvalidValue;
        } else {
            return OptionError.UnknownOption;
        }
    }
};

fn parseBool(s: []const u8) ?bool {
    if (std.ascii.eqlIgnoreCase(s, "true")) return true;
    if (std.ascii.eqlIgnoreCase(s, "false")) return false;
    if (std.ascii.eqlIgnoreCase(s, "1")) return true;
    if (std.ascii.eqlIgnoreCase(s, "0")) return false;
    return null;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test "EngineConfig defaults" {
    const cfg = EngineConfig{};
    try std.testing.expectEqual(true, cfg.use_opening_book);
    try std.testing.expectEqual(@as(u32, 16), cfg.hash_mb);
    try std.testing.expectEqual(@as(u8, 1), cfg.threads);
    try std.testing.expectEqual(@as(u32, 30), cfg.move_overhead_ms);
    try std.testing.expectEqualStrings("", cfg.syzygy_path);
    try std.testing.expectEqual(@as(u8, 1), cfg.syzygy_probe_depth);
    try std.testing.expectEqual(@as(u8, 7), cfg.syzygy_probe_limit);
    try std.testing.expectEqual(true, cfg.syzygy_50_move_rule);
}

test "setOption valid each" {
    var cfg = EngineConfig{};
    try cfg.setOption("Threads", "4");
    try std.testing.expectEqual(@as(u8, 4), cfg.threads);
    try cfg.setOption("Hash", "64");
    try std.testing.expectEqual(@as(u32, 64), cfg.hash_mb);
    try cfg.setOption("MoveOverhead", "100");
    try std.testing.expectEqual(@as(u32, 100), cfg.move_overhead_ms);
    try cfg.setOption("OwnBook", "false");
    try std.testing.expectEqual(false, cfg.use_opening_book);
    try cfg.setOption("OwnBook", "true");
    try std.testing.expectEqual(true, cfg.use_opening_book);
    try cfg.setOption("Clear Hash", null);
    try cfg.setOption("Clear Hash", "ignored");
    try cfg.setOption("SyzygyPath", "/tmp");
    try std.testing.expectEqualStrings("/tmp", cfg.syzygy_path);
    try cfg.setOption("SyzygyPath", "");
    try std.testing.expectEqualStrings("", cfg.syzygy_path);
    try cfg.setOption("SyzygyPath", null);
    try std.testing.expectEqualStrings("", cfg.syzygy_path);
    try cfg.setOption("SyzygyProbeDepth", "10");
    try std.testing.expectEqual(@as(u8, 10), cfg.syzygy_probe_depth);
    try cfg.setOption("SyzygyProbeLimit", "5");
    try std.testing.expectEqual(@as(u8, 5), cfg.syzygy_probe_limit);
    try cfg.setOption("Syzygy50MoveRule", "false");
    try std.testing.expectEqual(false, cfg.syzygy_50_move_rule);
}

test "setOption invalid values" {
    var cfg = EngineConfig{};
    try std.testing.expectError(OptionError.InvalidValue, cfg.setOption("Threads", "0"));
    try std.testing.expectError(OptionError.InvalidValue, cfg.setOption("Threads", "17"));
    try std.testing.expectError(OptionError.InvalidValue, cfg.setOption("Threads", "abc"));
    try std.testing.expectError(OptionError.InvalidValue, cfg.setOption("Hash", "0"));
    try std.testing.expectError(OptionError.InvalidValue, cfg.setOption("Hash", "1025"));
    try std.testing.expectError(OptionError.InvalidValue, cfg.setOption("MoveOverhead", "5001"));
    try std.testing.expectError(OptionError.InvalidValue, cfg.setOption("MoveOverhead", "-1"));
    try std.testing.expectError(OptionError.InvalidValue, cfg.setOption("OwnBook", "maybe"));
    try std.testing.expectError(OptionError.InvalidValue, cfg.setOption("SyzygyProbeDepth", "0"));
    try std.testing.expectError(OptionError.InvalidValue, cfg.setOption("SyzygyProbeDepth", "101"));
    try std.testing.expectError(OptionError.InvalidValue, cfg.setOption("SyzygyProbeLimit", "8"));
    try std.testing.expectError(OptionError.InvalidValue, cfg.setOption("SyzygyProbeLimit", "-1"));
    try std.testing.expectError(OptionError.InvalidValue, cfg.setOption("Syzygy50MoveRule", "yes"));
}

test "setOption missing value on spin" {
    var cfg = EngineConfig{};
    try std.testing.expectError(OptionError.MissingValue, cfg.setOption("Threads", null));
    try std.testing.expectError(OptionError.MissingValue, cfg.setOption("Hash", null));
    try std.testing.expectError(OptionError.MissingValue, cfg.setOption("MoveOverhead", null));
    try std.testing.expectError(OptionError.MissingValue, cfg.setOption("SyzygyProbeDepth", null));
}

test "setOption unknown" {
    var cfg = EngineConfig{};
    try std.testing.expectError(OptionError.UnknownOption, cfg.setOption("Bogus", "x"));
    try std.testing.expectError(OptionError.UnknownOption, cfg.setOption("Ponder", "true"));
}

test "option_decls covers expected names" {
    var found_threads = false;
    var found_hash = false;
    var found_moveoverhead = false;
    var found_ownbook = false;
    var found_clear = false;
    var found_syzygy_path = false;
    var found_probe_depth = false;
    var found_probe_limit = false;
    var found_50 = false;
    for (option_decls) |decl| {
        if (std.mem.eql(u8, decl.name, "Threads")) found_threads = true;
        if (std.mem.eql(u8, decl.name, "Hash")) found_hash = true;
        if (std.mem.eql(u8, decl.name, "MoveOverhead")) found_moveoverhead = true;
        if (std.mem.eql(u8, decl.name, "OwnBook")) found_ownbook = true;
        if (std.mem.eql(u8, decl.name, "Clear Hash")) found_clear = true;
        if (std.mem.eql(u8, decl.name, "SyzygyPath")) found_syzygy_path = true;
        if (std.mem.eql(u8, decl.name, "SyzygyProbeDepth")) found_probe_depth = true;
        if (std.mem.eql(u8, decl.name, "SyzygyProbeLimit")) found_probe_limit = true;
        if (std.mem.eql(u8, decl.name, "Syzygy50MoveRule")) found_50 = true;
    }
    try std.testing.expect(found_threads);
    try std.testing.expect(found_hash);
    try std.testing.expect(found_moveoverhead);
    try std.testing.expect(found_ownbook);
    try std.testing.expect(found_clear);
    try std.testing.expect(found_syzygy_path);
    try std.testing.expect(found_probe_depth);
    try std.testing.expect(found_probe_limit);
    try std.testing.expect(found_50);
    try std.testing.expectEqual(@as(usize, 9), option_decls.len);
}

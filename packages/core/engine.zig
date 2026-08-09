const std = @import("std");
const config_mod = @import("config.zig");
const tt_mod = @import("tt.zig");

pub const EngineConfig = config_mod.EngineConfig;
pub const OptionError = config_mod.OptionError;
pub const TranspositionTable = tt_mod.TranspositionTable;

pub const Engine = struct {
    allocator: std.mem.Allocator,
    config: EngineConfig,
    tt: TranspositionTable,
    // owned copy of syzygy_path when set via setOption/applyConfig
    syzygy_owned: ?[]u8 = null,

    pub fn init(allocator: std.mem.Allocator, cfg: EngineConfig) !Engine {
        const size_bytes: usize = @as(usize, cfg.hash_mb) * 1024 * 1024;
        var tt = try TranspositionTable.init(allocator, size_bytes);
        errdefer tt.deinit(allocator);
        var owned: ?[]u8 = null;
        var stored_cfg = cfg;
        if (cfg.syzygy_path.len > 0) {
            owned = try allocator.dupe(u8, cfg.syzygy_path);
            stored_cfg.syzygy_path = owned.?;
        }
        return .{ .allocator = allocator, .config = stored_cfg, .tt = tt, .syzygy_owned = owned };
    }

    pub fn deinit(self: *Engine) void {
        self.tt.deinit(self.allocator);
        if (self.syzygy_owned) |old| {
            self.allocator.free(old);
            self.syzygy_owned = null;
        }
    }

    pub fn applyConfig(self: *Engine, new_cfg: EngineConfig) !void {
        // Handle TT resize only when hash_mb changes
        if (new_cfg.hash_mb != self.config.hash_mb) {
            const new_tt = try TranspositionTable.init(self.allocator, @as(usize, new_cfg.hash_mb) * 1024 * 1024);
            self.tt.deinit(self.allocator);
            self.tt = new_tt;
        }
        // Handle syzygy_path ownership
        const new_path = new_cfg.syzygy_path;
        const cur_path = self.config.syzygy_path;
        if (!std.mem.eql(u8, new_path, cur_path)) {
            if (self.syzygy_owned) |old| {
                self.allocator.free(old);
                self.syzygy_owned = null;
            }
            if (new_path.len > 0) {
                const duped = try self.allocator.dupe(u8, new_path);
                self.syzygy_owned = duped;
                var tmp = new_cfg;
                tmp.syzygy_path = duped;
                self.config = tmp;
            } else {
                var tmp = new_cfg;
                tmp.syzygy_path = "";
                self.config = tmp;
            }
        } else {
            // path unchanged (same content) — preserve owned pointer
            const saved = self.config.syzygy_path;
            self.config = new_cfg;
            self.config.syzygy_path = saved;
        }
    }

    pub fn setOption(self: *Engine, name: []const u8, value: ?[]const u8) !void {
        // SyzygyPath needs owned allocation
        if (std.mem.eql(u8, name, "SyzygyPath")) {
            if (self.syzygy_owned) |old| {
                self.allocator.free(old);
                self.syzygy_owned = null;
            }
            if (value) |v| {
                const trimmed = std.mem.trim(u8, v, &std.ascii.whitespace);
                if (trimmed.len == 0) {
                    self.config.syzygy_path = "";
                    return;
                } else {
                    const duped = try self.allocator.dupe(u8, trimmed);
                    self.syzygy_owned = duped;
                    self.config.syzygy_path = duped;
                    return;
                }
            } else {
                self.config.syzygy_path = "";
                return;
            }
        }
        const old_hash = self.config.hash_mb;
        try self.config.setOption(name, value);
        if (self.config.hash_mb != old_hash) {
            const new_tt = try TranspositionTable.init(self.allocator, @as(usize, self.config.hash_mb) * 1024 * 1024);
            self.tt.deinit(self.allocator);
            self.tt = new_tt;
        }
    }

    pub fn clearHash(self: *Engine) void {
        self.tt.clear();
    }

    pub fn newGame(self: *Engine) void {
        self.tt.clear();
        // config unchanged
    }
};

test "engine init sizes TT to hash_mb" {
    const alloc = std.testing.allocator;
    var eng = try Engine.init(alloc, .{ .hash_mb = 1 });
    defer eng.deinit();
    try std.testing.expectEqual(@as(u32, 1), eng.config.hash_mb);
    try std.testing.expect(eng.tt.entries.len > 0);
}

test "engine applyConfig same hash_mb keeps TT" {
    const alloc = std.testing.allocator;
    var eng = try Engine.init(alloc, .{ .hash_mb = 1 });
    defer eng.deinit();
    const old_ptr = eng.tt.entries.ptr;
    const old_len = eng.tt.entries.len;
    // apply config with same hash_mb but different threads
    try eng.applyConfig(.{ .hash_mb = 1, .threads = 4 });
    try std.testing.expectEqual(@as(u8, 4), eng.config.threads);
    try std.testing.expectEqual(old_ptr, eng.tt.entries.ptr);
    try std.testing.expectEqual(old_len, eng.tt.entries.len);
}

test "engine applyConfig new hash_mb reallocates" {
    const alloc = std.testing.allocator;
    var eng = try Engine.init(alloc, .{ .hash_mb = 1 });
    defer eng.deinit();
    const old_ptr = eng.tt.entries.ptr;
    try eng.applyConfig(.{ .hash_mb = 2 });
    try std.testing.expectEqual(@as(u32, 2), eng.config.hash_mb);
    // reallocation should change buffer (or at least size)
    try std.testing.expect(eng.tt.entries.ptr != old_ptr or eng.tt.entries.len != 0);
    // size should be roughly double (power of two rounding may keep same? but 1MB vs 2MB should double entries)
    // Check entries len increased
    var eng2 = try Engine.init(alloc, .{ .hash_mb = 1 });
    defer eng2.deinit();
    try std.testing.expect(eng.tt.entries.len > eng2.tt.entries.len or eng.tt.entries.len == eng2.tt.entries.len * 2);
}

test "engine newGame clears TT and keeps config" {
    const alloc = std.testing.allocator;
    var eng = try Engine.init(alloc, .{ .hash_mb = 1, .threads = 4 });
    defer eng.deinit();
    // store something
    eng.tt.store(0x1234, 3, .exact, 100, null);
    try std.testing.expect(eng.tt.probe(0x1234) != null);
    eng.newGame();
    try std.testing.expect(eng.tt.probe(0x1234) == null);
    try std.testing.expectEqual(@as(u8, 4), eng.config.threads);
    try std.testing.expectEqual(@as(u32, 1), eng.config.hash_mb);
}

test "engine setOption Hash 64 and TT stats size changed" {
    const alloc = std.testing.allocator;
    var eng = try Engine.init(alloc, .{ .hash_mb = 16 });
    defer eng.deinit();
    const before = eng.tt.entries.len;
    try eng.setOption("Hash", "64");
    try std.testing.expectEqual(@as(u32, 64), eng.config.hash_mb);
    try std.testing.expect(eng.tt.entries.len > before);
}

test "engine setOption non-Hash does not reallocate" {
    const alloc = std.testing.allocator;
    var eng = try Engine.init(alloc, .{ .hash_mb = 1 });
    defer eng.deinit();
    const old_ptr = eng.tt.entries.ptr;
    try eng.setOption("Threads", "4");
    try std.testing.expectEqual(@as(u8, 4), eng.config.threads);
    try std.testing.expectEqual(old_ptr, eng.tt.entries.ptr);
}

test "engine setOption SyzygyPath owned" {
    const alloc = std.testing.allocator;
    var eng = try Engine.init(alloc, .{});
    defer eng.deinit();
    try eng.setOption("SyzygyPath", "/tmp");
    try std.testing.expectEqualStrings("/tmp", eng.config.syzygy_path);
    try eng.setOption("SyzygyPath", "");
    try std.testing.expectEqualStrings("", eng.config.syzygy_path);
}

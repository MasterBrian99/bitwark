const std = @import("std");
const core = @import("bitwark_core");
const busy_mod = @import("busy.zig");

pub const Controller = struct {
    busy: busy_mod.BusyState = .{},
    // we run search synchronously. Later this becomes a thread + CancellationToken.
    // The busy flag protects against `go` while searching.

    pub fn isBusy(self: Controller) bool {
        return !self.busy.canStartSearch();
    }

    pub fn startSearch(self: *Controller) bool {
        if (!self.busy.canStartSearch()) return false;
        self.busy.setSearching(true);
        return true;
    }

    pub fn endSearch(self: *Controller) void {
        self.busy.setSearching(false);
    }
};

test "controller busy" {
    var c = Controller{};
    try std.testing.expect(!c.isBusy());
    try std.testing.expect(c.startSearch());
    try std.testing.expect(c.isBusy());
    try std.testing.expect(!c.startSearch()); // second go while busy fails
    c.endSearch();
    try std.testing.expect(!c.isBusy());
}

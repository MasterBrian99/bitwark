const std = @import("std");

// Busy-state policy: if engine is searching, new `go` should be rejected or queued.
//  we use simple reject: `isSearching` → `busy`.
// Later this can become queued or best-move-pending.
pub const BusyState = struct {
    is_searching: bool = false,

    pub fn canStartSearch(self: BusyState) bool {
        return !self.is_searching;
    }

    pub fn setSearching(self: *BusyState, v: bool) void {
        self.is_searching = v;
    }
};

test "busy state" {
    var b = BusyState{};
    try std.testing.expect(b.canStartSearch());
    b.setSearching(true);
    try std.testing.expect(!b.canStartSearch());
}

pub const uci = @import("uci.zig");
pub const session = @import("session.zig");
pub const busy = @import("busy.zig");
pub const events = @import("events.zig");
pub const table = @import("table.zig");
pub const controller = @import("controller.zig");

test {
    _ = @import("uci.zig");
    _ = @import("session.zig");
    _ = @import("busy.zig");
    _ = @import("events.zig");
    _ = @import("table.zig");
    _ = @import("controller.zig");
}

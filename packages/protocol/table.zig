const std = @import("std");
const core = @import("bitwark_core");

// Packed-table allocation owned by protocol, layout owned by core.
// For we just allocate a core TT and expose it.

pub fn allocTT(allocator: std.mem.Allocator, size_bytes: usize) !core.tt.TranspositionTable {
    return core.tt.TranspositionTable.init(allocator, size_bytes);
}

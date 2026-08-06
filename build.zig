const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    // const mod = b.addModule("bitwark", .{
    //     .root_source_file = b.path("src/root.zig"),
    //     .target = target,
    // });

    const core_mod = b.addModule("core", .{
        .root_source_file = b.path("packages/core/lib.zig"),
        .target = target,
        .optimize = optimize,
    });

    const AppSpec = struct {
        name: []const u8,
        dir: []const u8,
        uses_protocol: bool,
    };
    const apps = [_]AppSpec{
        .{ .name = "bitwark", .dir = "bitwark", .uses_protocol = false },
        .{ .name = "bitwark-bench", .dir = "bitwark-bench", .uses_protocol = false },
        // .{ .name = "bitwark-dump", .dir = "bitwark-dump", .uses_protocol = false },
        // .{ .name = "bitwark-perft", .dir = "bitwark-perft", .uses_protocol = false },
        // .{ .name = "bitwark-eval", .dir = "bitwark-eval", .uses_protocol = false },
        // .{ .name = "bitwark-replay", .dir = "bitwark-replay", .uses_protocol = false },
        // .{ .name = "bitwark-cli", .dir = "bitwark-cli", .uses_protocol = false },
        // .{ .name = "bitwark-selfplay", .dir = "bitwark-selfplay", .uses_protocol = false },
        // .{ .name = "bitwarkd", .dir = "bitwarkd", .uses_protocol = true },
    };
    const run_step = b.step("run", "Run the main bitwark executable");
    for (apps) |app| {
        const src_path = std.fmt.allocPrint(b.allocator, "apps/{s}/main.zig", .{app.dir}) catch @panic("OOM");

        const app_mod = b.createModule(.{
            .root_source_file = b.path(src_path),
            .target = target,
            .optimize = optimize,
        });
        if (app.uses_protocol) {
            // app_mod.addImport("bitwark_protocol", protocol_mod);
        } else {
            app_mod.addImport("bitwark_core", core_mod);
        }

        const exe = b.addExecutable(.{
            .name = app.name,
            .root_module = app_mod,
        });
        b.installArtifact(exe);

        if (std.mem.eql(u8, app.name, "bitwark")) {
            const run_cmd = b.addRunArtifact(exe);
            run_cmd.step.dependOn(b.getInstallStep());
            if (b.args) |args| run_cmd.addArgs(args);
            run_step.dependOn(&run_cmd.step);
        }
    }
    //   const core_tests = b.addTest(.{ .root_module = core_mod });
    // test_step.dependOn(&b.addRunArtifact(core_tests).step);

    // const protocol_tests = b.addTest(.{ .root_module = protocol_mod });
    // test_step.dependOn(&b.addRunArtifact(protocol_tests).step);
}

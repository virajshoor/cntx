// Compile and link the C core through Cargo. Only checked-in C sources are
// compiled; builds never download tools. Targets: macOS and Linux.
fn main() {
    if !matches!(
        std::env::var("CARGO_CFG_TARGET_OS").as_deref(),
        Ok("macos" | "linux")
    ) {
        panic!("cntx supports macOS and Linux; the C core requires a Unix toolchain");
    }
    let mut build = cc::Build::new();
    build
        .file("csrc/agent.c")
        .file("csrc/context.c")
        .file("csrc/permissions.c")
        .file("csrc/routing.c")
        .file("csrc/tools.c")
        .include("csrc")
        .std("c17")
        .define("_POSIX_C_SOURCE", "200809L")
        .flag_if_supported("-Wall")
        .flag_if_supported("-Wextra")
        .flag_if_supported("-Wpedantic");
    println!("cargo:rerun-if-changed=csrc/cntx.h");
    for source in [
        "csrc/agent.c",
        "csrc/context.c",
        "csrc/permissions.c",
        "csrc/routing.c",
        "csrc/tools.c",
    ] {
        println!("cargo:rerun-if-changed={source}");
    }
    build.compile("cntxcore");
}

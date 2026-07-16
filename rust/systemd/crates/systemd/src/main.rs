fn main() {
    let exec_name = std::env::args()
        .next()
        .expect("could not get executable name from args");
    // The kernel execs PID 1 under whatever name the bootloader/stage-2 wiring
    // chose — `systemd`, `/init`, `/sbin/init`, … Recent nixpkgs boots stage-2
    // by exec'ing the init binary directly, so argv[0] is `init`, not a
    // `…/systemd` path. Like upstream systemd, the definitive signal that we
    // are the system manager is being PID 1, so dispatch on that first (a name
    // match still covers being started as `systemd` from a non-PID-1 context).
    let is_pid1 = std::process::id() == 1;
    if exec_name.ends_with("exec_helper") {
        libsystemd::entrypoints::run_exec_helper();
    } else if is_pid1
        || exec_name.ends_with("rust-systemd")
        || exec_name.ends_with("systemd_rs")
        || exec_name.ends_with("systemd")
    {
        libsystemd::entrypoints::run_service_manager();
    } else {
        eprintln!(
            "Can only start as systemd, rust-systemd, or exec_helper, or as PID 1. Was: {exec_name}"
        );
    }
}

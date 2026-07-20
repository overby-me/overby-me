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
    // `systemd --user` starts a per-user manager instance (never PID 1). Check
    // this before the system-manager name match, since argv[0] is still
    // `…/systemd` in that case.
    let is_user = !is_pid1 && std::env::args().any(|a| a == "--user");
    if exec_name.ends_with("exec_helper") {
        libsystemd::entrypoints::run_exec_helper();
    } else if is_user {
        libsystemd::entrypoints::run_user_manager();
    } else if is_pid1
        || exec_name.ends_with("rust-systemd")
        || exec_name.ends_with("systemd_rs")
        || exec_name.ends_with("systemd")
        // Being invoked as `init` (basename) is the system-manager signal too:
        // systemd-nspawn runs a container payload it doesn't recognise as
        // systemd *as PID 2* (with its own stub as PID 1), exec'ing the
        // container's `/sbin/init` (here a symlink to us) under argv[0]
        // `/bin/init`. In that case is_pid1 is false, so match the name.
        || std::path::Path::new(&exec_name)
            .file_name()
            .is_some_and(|s| s == "init")
    {
        libsystemd::entrypoints::run_service_manager();
    } else {
        eprintln!(
            "Can only start as systemd, rust-systemd, or exec_helper, or as PID 1. Was: {exec_name}"
        );
    }
}

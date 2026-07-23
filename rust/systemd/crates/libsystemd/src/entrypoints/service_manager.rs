use log::{error, info, trace, warn};
use signal_hook::iterator::Signals;
use std::sync::{Arc, Mutex, RwLock};

use crate::config;
use crate::control;
use crate::generators;
use crate::lock_ext::RwLockExt;
use crate::logging;
use crate::notification_handler;
use crate::platform;
use crate::runtime_info;
use crate::signal_handler;
use crate::socket_activation;
use crate::units;

/// Emit an early-boot progress line to `/dev/kmsg`, the way upstream systemd
/// logs before journald/console are up. Invaluable in the initrd, where normal
/// logging has nowhere to go yet — these lines show up in `dmesg`/console.
/// Best-effort: silently ignored if `/dev/kmsg` isn't available.
pub fn kmsg(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open("/dev/kmsg") {
        let _ = writeln!(f, "<6>rust-systemd[1]: {msg}");
    }
}

/// Install a panic hook that routes the panic message and location to
/// `/dev/kmsg` before chaining to the default hook. As PID 1 in early boot,
/// and especially in stage-2 right after switch-root, stderr is not connected
/// to `/dev/console`, so a panic on the main thread would otherwise vanish and
/// surface only as the kernel's "Attempted to kill init" panic with no cause.
/// Routing panics to kmsg makes a PID 1 crash diagnosable from the boot log.
fn install_kmsg_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        kmsg(&format!("PANIC: {info}"));
        default_hook(info);
    }));
}

pub fn run_service_manager() {
    install_kmsg_panic_hook();
    pid1_specific_setup();
    let in_initrd = config::in_initrd();
    kmsg(&format!(
        "service manager starting (pid={}, in_initrd={in_initrd})",
        std::process::id(),
    ));

    // Put SYSTEMD_IN_INITRD in PID 1's own environment, like upstream systemd,
    // so every child inherits it: generators AND services. The initrd's
    // initrd-parse-etc.service runs systemd-sysroot-fstab-check, which refuses
    // to run (exit 1 → emergency.target) unless SYSTEMD_IN_INITRD=1 is set.
    // Safe here: still single-threaded this early in PID 1 startup.
    if in_initrd {
        unsafe { std::env::set_var("SYSTEMD_IN_INITRD", "1") };
    }

    // Apply ManagerEnvironment= from system.conf to PID 1's own environment,
    // like upstream systemd, so generators and services inherit it. NixOS's
    // systemd-initrd passes SYSTEMD_SYSROOT_FSTAB (the real root's fstab) to the
    // fstab-generator exclusively through this directive; without it the
    // generator never emits sysroot.mount and the initrd can't mount the real
    // root. Safe here: still single-threaded this early in PID 1 startup.
    crate::control::apply_manager_environment_to_process();
    if let (true, Some(v)) = (in_initrd, std::env::var_os("SYSTEMD_SYSROOT_FSTAB")) {
        kmsg(&format!(
            "ManagerEnvironment: SYSTEMD_SYSROOT_FSTAB={}",
            v.to_string_lossy()
        ));
    }

    let cli_args = CliArgs::try_parse().unwrap_or_else(|e| {
        unrecoverable_error(e.to_string());
        unreachable!();
    });

    // Ensure runtime unit directories exist so they appear in the unit search
    // path (config filters directories by existence at load time).
    let _ = std::fs::create_dir_all("/run/systemd/system");
    let _ = std::fs::create_dir_all("/run/systemd/transient");
    // The userdb runtime directory is where user-database services drop their
    // varlink sockets. Tools (and tests) expect it to exist even when no such
    // service is running, matching upstream systemd.
    let _ = std::fs::create_dir_all("/run/systemd/userdb");

    let (log_conf, mut conf) = config::load_config();
    kmsg(&format!(
        "config loaded: target={}, {} unit dir(s)",
        conf.target_unit,
        conf.unit_dirs.len()
    ));

    logging::setup_logging(&log_conf).unwrap();

    // Apply PID 1's own NUMA memory policy from [Manager] NUMAPolicy= now, on
    // TID 1 while still single-threaded (set_mempolicy(2) affects the calling
    // task). daemon-reload later re-applies it via the main-thread loop below.
    crate::control::apply_manager_numa_policy();

    // Log the selected boot target — especially useful when emergency/rescue
    // mode was requested via kernel command line.
    let target = &conf.target_unit;
    if target == "emergency.target" {
        eprintln!(
            "rust-systemd: EMERGENCY MODE — booting to emergency.target (requested via kernel command line)"
        );
        info!("Emergency mode requested — booting to emergency.target");
    } else if target == "rescue.target" {
        eprintln!(
            "rust-systemd: RESCUE MODE — booting to rescue.target (requested via kernel command line)"
        );
        info!("Rescue mode requested — booting to rescue.target");
    } else if target != "default.target" {
        eprintln!("rust-systemd: boot target overridden to {target} (via kernel command line)");
        info!("Boot target overridden to {target} via kernel command line");
    } else {
        info!("Booting to default target: {target}");
    }

    // Augment PATH with binary directories derived from the unit search
    // paths.  Many upstream systemd unit files use bare command names in
    // ExecStart= (e.g. `systemd-tmpfiles`, `udevadm`).  Real systemd
    // resolves these via compiled-in prefix paths; rust-systemd adds the
    // relevant package directories to PATH instead.
    config::augment_path_from_unit_dirs(&conf.unit_dirs);

    // Run external generators before loading units.
    //
    // Generators are small executables (e.g. systemd-gpt-auto-generator,
    // zram-generator) that dynamically create unit files at boot time.
    // They are called with three output directory arguments and can write
    // unit files, symlinks, and .wants/.requires directories.
    //
    // Built-in generators (fstab, getty) are skipped since rust-systemd has
    // native implementations.  The output directories are then inserted
    // into the unit search path at the correct priority positions.
    let generator_output = generators::run_generators(&conf.unit_dirs);
    generators::augment_unit_dirs_with_generators(&mut conf.unit_dirs, &generator_output);

    #[cfg(feature = "cgroups")]
    {
        // Non-fatal: PID 1 must not die if the cgroup move fails (e.g. the
        // cgroup2 hierarchy could not be mounted). mount_api_filesystems()
        // mounts /sys/fs/cgroup during pid1_specific_setup, so this normally
        // succeeds.
        if let Err(e) =
            platform::cgroups::move_to_own_cgroup(&std::path::PathBuf::from("/sys/fs/cgroup"))
        {
            log::warn!("could not move to own cgroup: {e}");
        }
    }

    // TODO make configurable
    let should_go_to_new_session = false;
    if should_go_to_new_session && !move_to_new_session() {
        return;
    }

    crate::platform::become_subreaper(true);

    let run_info = prepare_runtimeinfo(&conf, cli_args.dry_run);

    // After daemon-reexec (execve), the process inherits the signal mask from
    // the previous instance.  signal_hook blocks SIGCHLD/etc in all threads so
    // only the iterator thread receives them.  After execve the single
    // remaining thread still has those signals blocked, so all threads spawned
    // from it inherit the mask and SIGCHLD is never delivered.  Reset the
    // signal mask to empty before registering handlers.
    {
        let empty = nix::sys::signal::SigSet::empty();
        let _ = nix::sys::signal::sigprocmask(
            nix::sys::signal::SigmaskHow::SIG_SETMASK,
            Some(&empty),
            None,
        );
    }

    // Build the set of signals to listen for: standard signals + SIGRTMIN+N
    // real-time signals used for target switching, shutdown, reexec, etc.
    let mut sig_list = vec![
        signal_hook::consts::SIGCHLD,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGQUIT,
    ];
    sig_list.extend(signal_handler::sigrtmin_signals());

    let signals = match Signals::new(sig_list) {
        Ok(signals) => signals,
        Err(e) => {
            unrecoverable_error(format!("Couldnt setup listening to the signals: {e}"));
            // unrecoverable_error always shutsdown rust-systemd
            unreachable!("");
        }
    };
    // listen to signals
    let _signal_handle = start_signal_handler_thread(signals, run_info.clone());

    // If this is a daemon-reexec, restore PID tracking for running services.
    // This must happen after the signal handler is running (so SIGCHLD is
    // caught) but before we start activating new units.
    let is_reexec = signal_handler::check_and_restore_reexec_state(&run_info);
    if is_reexec {
        info!("Resumed after daemon-reexec");
    }

    // listen on user commands like listunits/kill/restart...
    control::open_all_sockets(run_info.clone(), &conf);

    start_notification_handler_thread(run_info.clone());
    start_stdout_handler_thread(run_info.clone());
    start_stderr_handler_thread(run_info.clone());

    socket_activation::start_socketactivation_thread(run_info.clone());

    crate::timer_scheduler::start_timer_scheduler_thread(run_info.clone());
    crate::path_watcher::start_path_watcher_thread(run_info.clone());
    crate::watchdog::start_watchdog_thread(run_info.clone());
    crate::dbus_server::start_dbus_server_thread(run_info.clone());
    // Track manual mount(8) operations via /proc/self/mountinfo and synthesise
    // active `.mount` units for them (systemctl is-active <path>.mount).
    crate::units::start_mount_monitor_thread(run_info.clone());

    // Rebuild synthetic `.device` units from the udev db.  Device
    // units are created on-the-fly from `udev-event` RPC notifications
    // and are NOT persisted in unit files, so after a daemon-reexec
    // (or fresh boot where udev has already written its db) the in-
    // memory unit table is missing them.  Walking /run/udev/data/*
    // re-populates the table without waiting for udevd to re-broadcast.
    // Runs AFTER all helper threads are up so Wants= activations
    // triggered by the replay can reach the notification handler etc.
    // Matches upstream's `manager_enumerate_devices()` at boot.
    crate::units::rebuild_device_units_from_udev_db(&run_info);
    // Native block-device coldplug: probe block devices directly (ext*
    // superblock) to synthesize referenced by-label/by-uuid .device units even
    // when systemd-udevd hasn't pushed the event yet. Critical in the initrd,
    // where the root device is referenced by label and udevd's push can stall.
    crate::units::coldplug_referenced_block_devices(&run_info);
    // Coldplug fallback: synthesize plugged `.device` units for referenced
    // devices whose nodes already exist in /dev. After switch-root the real
    // root's /dev is fully populated but the udev db was cleared and stage-2's
    // udev-trigger may not have (re)notified PID 1 yet, so this ensures
    // referenced devices (console, root) are present before activation instead
    // of waiting on an unreliable udevd push.
    crate::units::synthesize_referenced_present_devices(&run_info);
    // Some referenced devices (notably the root block device in the initrd)
    // only appear once their driver probes during activation, which runs AFTER
    // this point. Re-run the coldplug in the background (non-blocking, so
    // activation can proceed and load those drivers) so their .device units are
    // synthesized as soon as the devices show up, unblocking the mounts that
    // wait on them. Best-effort with a bounded lifetime.
    {
        let ri = run_info.clone();
        let _ = std::thread::Builder::new()
            .name("coldplug-retry".to_owned())
            .spawn(move || {
                for _ in 0..120 {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    crate::units::coldplug_referenced_block_devices(&ri);
                    crate::units::synthesize_referenced_present_devices(&ri);
                }
            });
    }
    // Cleanup the reexec .status file now that rebuild has consumed
    // it (see note in check_and_restore_reexec_state about the
    // deferred deletion).
    let _ = std::fs::remove_file("/run/systemd/rust-systemd-reexec-state.status");

    trace!("Started all helper threads. Start activating units");

    let target_id: units::UnitId = {
        let run_info_guard: &runtime_info::RuntimeInfo = &run_info.read_poisoned();
        let target_name = &run_info_guard.config.target_unit;
        // The target unit might have been removed as a symlink alias
        // (e.g. default.target -> multi-user.target). Look it up by name
        // first, then fall back to checking aliases on other units.
        if let Some(unit) = run_info_guard
            .unit_table
            .values()
            .find(|u| u.id.name == *target_name)
        {
            unit.id.clone()
        } else if let Some(unit) = run_info_guard
            .unit_table
            .values()
            .find(|u| u.common.unit.aliases.iter().any(|a| a == target_name))
        {
            unit.id.clone()
        } else {
            use std::convert::TryInto;
            target_name.as_str().try_into().unwrap()
        }
    };

    // parallel startup of all services
    if is_reexec {
        // After daemon-reexec, unit statuses were already restored from the
        // reexec state file by check_and_restore_reexec_state (PIDs for
        // running services + status for all units that existed before reexec).
        // Skip full activation — it would block threads waiting for READY=1
        // from notify services that already sent it to the old instance.
        info!("daemon-reexec: skipped full activation, statuses restored from state file");
    } else {
        kmsg(&format!("activating target {}", target_id.name));
        let _ = in_initrd;
        units::set_active_goal(&target_id.name);
        spawn_active_goal_redrive(run_info.clone());
        units::activate_needed_units(target_id, run_info);
    }

    kmsg("initial activation returned; entering main-thread task loop");
    // The main thread (TID 1) now services tasks that must run on PID 1 itself:
    // re-applying the Manager NUMA memory policy on `daemon-reload`.
    // set_mempolicy(2) affects the calling task, and the reload runs on a worker
    // thread which hands the re-apply here (blocking until it completes). This
    // blocks the main thread the way the previous `handle.join()` did; process
    // shutdown is driven by the signal-handler thread, so it runs until exit.
    control::run_manager_numa_reapply_loop();
}

/// Entry point for the per-user service manager (`systemd --user`).
///
/// Unlike [`run_service_manager`], this boots a lightweight manager for a
/// single user: it performs NO PID 1 / system setup (no API-filesystem
/// mounting, no cgroup-root move, no generators, no device coldplug) and does
/// NOT start the varlink server (its path is the hardcoded system socket
/// `/run/systemd/io.systemd.Manager`, which a user manager must never touch).
/// It reuses the shared machinery — signal handling, the control socket, the
/// notification/stdout/stderr handlers, socket activation and the
/// timer/path/watchdog threads — driven by a user-mode [`config::Config`] whose
/// paths live under `$XDG_RUNTIME_DIR` and the XDG user unit directories.
///
/// Started by `user@<uid>.service`; clients reach it via
/// `$XDG_RUNTIME_DIR/systemd/control.socket` (`systemctl --user`,
/// `systemd-run --user`).
pub fn run_user_manager() {
    // Mark this process — and its exec_helper children, which inherit the
    // environment — as a user manager, so ExecDirectory= handling uses the XDG
    // base directories ($HOME/.local/state, $HOME/.config, …) instead of the
    // system /var/lib, /etc, … Set while still single-threaded, since
    // std::env::set_var is not thread-safe.
    unsafe { std::env::set_var("SYSTEMD_USER_MANAGER", "1") };
    let conf = build_user_config();

    // A user manager logs to stderr; the journal captures it via user@.service.
    let log_conf = config::LoggingConfig {
        log_to_stdout: true,
        log_to_disk: false,
        log_dir: std::path::PathBuf::from("/dev/null"),
    };
    let _ = logging::setup_logging(&log_conf);

    info!(
        "systemd --user starting (uid={}, {} unit dir(s), runtime={})",
        nix::unistd::Uid::current().as_raw(),
        conf.unit_dirs.len(),
        conf.notification_sockets_dir.display(),
    );

    // Ensure the runtime dirs the manager writes into exist: the sockets dir
    // (holds the control socket) and the transient dir (`systemd-run --user`).
    let _ = std::fs::create_dir_all(&conf.notification_sockets_dir);
    let _ = std::fs::create_dir_all(conf.notification_sockets_dir.join("transient"));

    // A user manager reaps its own service descendants.
    crate::platform::become_subreaper(true);

    let run_info = prepare_runtimeinfo(&conf, false);

    // Reset the signal mask before registering handlers (see the identical note
    // in run_service_manager: an inherited non-empty mask would swallow SIGCHLD).
    {
        let empty = nix::sys::signal::SigSet::empty();
        let _ = nix::sys::signal::sigprocmask(
            nix::sys::signal::SigmaskHow::SIG_SETMASK,
            Some(&empty),
            None,
        );
    }

    let mut sig_list = vec![
        signal_hook::consts::SIGCHLD,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGQUIT,
    ];
    sig_list.extend(signal_handler::sigrtmin_signals());
    let signals = match Signals::new(sig_list) {
        Ok(s) => s,
        Err(e) => {
            error!("systemd --user: could not set up signal handling: {e}");
            return;
        }
    };
    let handle = start_signal_handler_thread(signals, run_info.clone());

    // Bind the user control socket ($XDG_RUNTIME_DIR/systemd/control.socket)
    // and serve control connections. Deliberately no varlink server here (see
    // the doc comment above).
    {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;
        let control_sock_path = conf.notification_sockets_dir.join("control.socket");
        if control_sock_path.exists() {
            let _ = std::fs::remove_file(&control_sock_path);
        }
        match UnixListener::bind(&control_sock_path) {
            Ok(unixsock) => {
                let _ = std::fs::set_permissions(
                    &control_sock_path,
                    std::fs::Permissions::from_mode(0o666),
                );
                control::accept_control_connections_unix_socket(run_info.clone(), unixsock);
            }
            Err(e) => {
                error!(
                    "systemd --user: failed to bind control socket {}: {e}",
                    control_sock_path.display()
                );
                return;
            }
        }
    }

    start_notification_handler_thread(run_info.clone());
    start_stdout_handler_thread(run_info.clone());
    start_stderr_handler_thread(run_info.clone());
    socket_activation::start_socketactivation_thread(run_info.clone());
    crate::timer_scheduler::start_timer_scheduler_thread(run_info.clone());
    crate::path_watcher::start_path_watcher_thread(run_info.clone());
    crate::watchdog::start_watchdog_thread(run_info.clone());

    // Signal readiness to the system manager. user@<uid>.service is
    // Type=notify-reload, so without this `systemctl start user@<uid>.service`
    // blocks for TimeoutStartSec (~90s) and is then killed. Send READY=1 now —
    // the control socket and handlers are up, so the manager can already serve
    // `systemctl --user` / `systemd-run --user` — rather than after the
    // possibly slow default.target activation below.
    sd_notify_user_manager("READY=1\n");

    // Activate the user's default.target if present. A user manager with no
    // default.target is still useful (it idles, serving transient units from
    // `systemd-run --user`), so a missing target must not be fatal.
    let target_name = conf.target_unit.clone();
    let target_id: Option<units::UnitId> = {
        let ri = run_info.read_poisoned();
        ri.unit_table
            .values()
            .find(|u| u.id.name == target_name)
            .or_else(|| {
                ri.unit_table
                    .values()
                    .find(|u| u.common.unit.aliases.iter().any(|a| a == &target_name))
            })
            .map(|u| u.id.clone())
    };
    if let Some(target_id) = target_id {
        units::set_active_goal(&target_id.name);
        spawn_active_goal_redrive(run_info.clone());
        units::activate_needed_units(target_id, run_info);
    } else {
        info!("systemd --user: no {target_name} present; idling for transient units");
    }

    handle.join().unwrap();
}

/// Build a user-mode [`config::Config`] rooted at the XDG base directories.
///
/// `notification_sockets_dir` becomes `$XDG_RUNTIME_DIR/systemd` so the control
/// socket lands at `$XDG_RUNTIME_DIR/systemd/control.socket`, matching where
/// `systemctl --user` / `systemd-run --user` look.
fn build_user_config() -> config::Config {
    let uid = nix::unistd::Uid::current().as_raw();
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("/run/user/{uid}"));
    let home = std::env::var("HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/root".to_owned());
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{home}/.config"));
    let self_path =
        std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("/proc/self/exe"));

    let sockets_dir = std::path::PathBuf::from(format!("{runtime_dir}/systemd"));

    // Create the runtime unit dirs this manager owns, so they exist before the
    // existence filter below and give `systemd-run --user` somewhere to drop
    // transient units.
    for sub in ["transient", "user.control", "user"] {
        let _ = std::fs::create_dir_all(sockets_dir.join(sub));
    }

    // User unit search dirs, highest priority first (mirrors systemd's user
    // manager search path): transient + runtime control units, then per-user
    // config, then system-wide user units. Only existing directories are kept —
    // the unit loader errors on a missing search path, and the optional
    // system-wide user dirs (/etc/systemd/user, /usr/lib/systemd/user, …) may be
    // absent.
    let unit_dirs: Vec<std::path::PathBuf> = [
        sockets_dir.join("transient"),
        sockets_dir.join("user.control"),
        std::path::PathBuf::from(format!("{config_home}/systemd/user")),
        std::path::PathBuf::from("/etc/systemd/user"),
        sockets_dir.join("user"),
        std::path::PathBuf::from("/usr/local/lib/systemd/user"),
        std::path::PathBuf::from("/usr/lib/systemd/user"),
    ]
    .into_iter()
    .filter(|p| p.is_dir())
    .collect();

    config::Config {
        unit_dirs,
        target_unit: "default.target".to_owned(),
        notification_sockets_dir: sockets_dir,
        self_path,
    }
}

/// Send an `sd_notify` message to `$NOTIFY_SOCKET`, if set.
///
/// A user manager is itself a service (`user@<uid>.service`) and must signal
/// readiness to the system manager that started it, exactly like any other
/// `Type=notify` service. Best-effort: a missing or unreachable socket is
/// silently ignored.
fn sd_notify_user_manager(msg: &str) {
    let Ok(sock_path) = std::env::var("NOTIFY_SOCKET") else {
        return;
    };
    // Abstract sockets are named with a leading '@' (mapped to a NUL byte);
    // rust-systemd uses filesystem paths, but handle both like sd_notify(3).
    let path = if let Some(stripped) = sock_path.strip_prefix('@') {
        format!("\0{stripped}")
    } else {
        sock_path
    };
    if let Ok(sock) = std::os::unix::net::UnixDatagram::unbound() {
        let _ = sock.send_to(msg.as_bytes(), &path);
    }
}

/// Background re-drive for the current activation goal.
///
/// rust-systemd's activation is a forward walk that can return before its goal
/// is reached: at a given moment no unit may be startable (all remaining ones
/// are blocked on asynchronous mount/device/oneshot completions), and the
/// targeted async re-drive paths re-evaluate the static boot target rather than
/// the CURRENT goal. After `systemctl isolate` (e.g. the initrd isolating to
/// `initrd-switch-root.target`, or any stage-2 target chain) that leaves the
/// new goal stalled nondeterministically. This thread periodically re-runs
/// activation for whatever goal is currently active (see
/// `units::set_active_goal`) until it is reached, so a unit that became active
/// asynchronously reliably unblocks its waiters. `activate_unit` is idempotent,
/// so re-drives that make no progress are cheap, and once the goal is active we
/// stop calling activation and just idle.
fn spawn_active_goal_redrive(run_info: runtime_info::ArcMutRuntimeInfo) {
    std::thread::spawn(move || {
        // Up to ~120s of re-drives; stops issuing activations once the goal is
        // active. In the initrd the switch-root execve tears this thread down.
        for _ in 0..400 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            // Never launch a concurrent activation: if the initial activation
            // (or another re-drive/control/exit-handler pass) is already
            // running, skip this tick. Each activate_needed_units spins up a
            // 32-thread pool; running several at once just contends locks and
            // starves real progress (observed: boot crawling, nothing starting).
            if units::activation_in_flight() {
                continue;
            }
            // Yield to a pending table-wide writer (daemon-reload).
            if units::writer_pending() {
                continue;
            }
            let Some(goal_name) = units::active_goal() else {
                continue;
            };
            let goal = {
                let ri = run_info.read_poisoned();
                ri.unit_table
                    .values()
                    .find(|u| u.id.name == goal_name || u.common.unit.aliases.contains(&goal_name))
                    .map(|u| (u.id.clone(), u.common.status.read_poisoned().is_started()))
            };
            match goal {
                // Goal reached — idle (a later isolate may set a new goal).
                Some((_, true)) | None => continue,
                Some((id, false)) => {
                    let _ = units::activate_needed_units(id, run_info.clone());
                }
            }
        }
    });
}

fn find_shell_path() -> Option<std::path::PathBuf> {
    let possible_paths = vec![
        std::path::PathBuf::from("/bin/sh"),
        std::path::PathBuf::from("/sbin/sh"),
        std::path::PathBuf::from("/usr/bin/sh"),
    ];

    // TODO make configurable
    possible_paths.into_iter().find(|p| p.exists())
}

fn unrecoverable_error(error: String) {
    if nix::unistd::getpid().as_raw() == 1 {
        eprintln!("Unrecoverable error: {error}");
        if let Some(shell_path) = find_shell_path() {
            match std::process::Command::new(shell_path).spawn() {
                Ok(mut child) => match child.wait() {
                    Ok(_) => {
                        let dur = std::time::Duration::from_secs(10);
                        eprintln!("Returned from shell. Will exit after sleeping: {dur:?}");
                        std::thread::sleep(dur);
                        std::process::exit(1);
                    }
                    Err(e) => {
                        let dur = std::time::Duration::from_secs(1_000_000);
                        eprintln!(
                            "Error while waiting on the shell: {e}. Will sleep for {dur:?} and then exit"
                        );
                        std::thread::sleep(dur);
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    let dur = std::time::Duration::from_secs(1_000_000);
                    eprintln!(
                        "Error while starting the shell: {e}. Will sleep for {dur:?} and then exit"
                    );
                    std::thread::sleep(dur);
                    std::process::exit(1);
                }
            }
        } else {
            let dur = std::time::Duration::from_secs(10);
            eprintln!("Cannot find a shell for emergency. Will sleep for {dur:?} and then exit");
            std::thread::sleep(dur);
            std::process::exit(1);
        }
    } else {
        panic!("{}", error);
    }
}

fn move_to_new_session() -> bool {
    match unsafe { nix::unistd::fork() } {
        Ok(nix::unistd::ForkResult::Child) => {
            nix::unistd::setsid().unwrap();
            true
        }
        Ok(nix::unistd::ForkResult::Parent { .. }) => false,
        Err(e) => {
            error!("Fork before setsid failed: {e}");
            false
        }
    }
}

/// Mount the core API filesystems the way upstream systemd's `mount_setup()`
/// does, so rust-systemd works when the kernel execs it as PID 1 in an
/// otherwise-bare environment — notably systemd-in-initrd, where nothing has
/// mounted `/proc`, `/sys`, `/dev`, `/run` or the cgroup hierarchy yet.
///
/// Idempotent: each mount is skipped if something is already mounted there
/// (`EBUSY`), so this is a no-op on a normal boot where stage-1 already set
/// these up, and any other failure is logged rather than fatal (PID 1 must not
/// die). The order matters — `/proc`, `/sys` and `/dev` first, then the mounts
/// that live under them.
#[cfg(target_os = "linux")]
fn mount_api_filesystems() {
    use nix::mount::{MsFlags, mount};

    let nsdev = MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV;
    // (source, target, fstype, flags, data)
    let table: &[(&str, &str, &str, MsFlags, Option<&str>)] = &[
        ("proc", "/proc", "proc", nsdev, None),
        ("sysfs", "/sys", "sysfs", nsdev, None),
        (
            "devtmpfs",
            "/dev",
            "devtmpfs",
            MsFlags::MS_NOSUID,
            Some("mode=755"),
        ),
        (
            "tmpfs",
            "/run",
            "tmpfs",
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
            Some("mode=755"),
        ),
        (
            "devpts",
            "/dev/pts",
            "devpts",
            MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC,
            Some("mode=620,gid=5"),
        ),
        (
            "tmpfs",
            "/dev/shm",
            "tmpfs",
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
            Some("mode=1777"),
        ),
        (
            "cgroup2",
            "/sys/fs/cgroup",
            "cgroup2",
            nsdev,
            Some("nsdelegate"),
        ),
    ];

    for (source, target, fstype, flags, data) in table {
        // Create the mount point. Entries under /dev and /sys come after their
        // parent in the table, so the parent is mounted by the time we get here.
        let _ = std::fs::create_dir_all(target);
        // Skip if something is already mounted here. Relying on EBUSY is wrong
        // for stackable filesystems: mounting a fresh `tmpfs` over an
        // already-mounted /run (or /dev/shm, …) SUCCEEDS and stacks an empty
        // tmpfs on top, shadowing whatever was there. In stage-2 that hides the
        // /run carried over from the initrd — including the NixOS activation's
        // /run/current-system, which /etc/profile needs for the system PATH.
        // Real systemd skips API mounts that are already present, so match that.
        if is_mount_point(target) {
            continue;
        }
        match mount(Some(*source), *target, Some(*fstype), *flags, *data) {
            Ok(()) => {}
            Err(nix::errno::Errno::EBUSY) => {} // already mounted — fine
            Err(e) => log::warn!("mount_api_filesystems: {target} ({fstype}): {e}"),
        }
    }
}

/// Whether `path` is a mount point — true if its device id differs from its
/// parent's. Used to avoid stacking a second mount over an already-mounted API
/// filesystem (e.g. re-mounting /run in stage-2 would shadow the initrd's /run).
#[cfg(target_os = "linux")]
/// Create the standard `/dev` symlinks that systemd's `dev_setup()` creates so
/// the global `/dev` provides `/dev/fd`, `/dev/stdin/stdout/stderr` and
/// `/dev/core`. Best-effort: only creates a link if it does not already exist.
fn create_standard_dev_symlinks() {
    let links = [
        ("/proc/self/fd", "/dev/fd"),
        ("/proc/self/fd/0", "/dev/stdin"),
        ("/proc/self/fd/1", "/dev/stdout"),
        ("/proc/self/fd/2", "/dev/stderr"),
        ("/proc/kcore", "/dev/core"),
    ];
    for (target, link) in links {
        if std::fs::symlink_metadata(link).is_err() {
            let _ = std::os::unix::fs::symlink(target, link);
        }
    }
}

fn is_mount_point(path: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    let p = std::path::Path::new(path);
    let Ok(meta) = std::fs::symlink_metadata(p) else {
        return false;
    };
    let parent = p.parent().unwrap_or_else(|| std::path::Path::new("/"));
    match std::fs::metadata(parent) {
        Ok(pmeta) => meta.dev() != pmeta.dev(),
        Err(_) => false,
    }
}

#[cfg(target_os = "linux")]
fn pid1_specific_setup() {
    if nix::unistd::getpid().as_raw() != 1 {
        return;
    }

    // Ensure the API filesystems are mounted before we touch /dev/console,
    // /proc, /sys or the cgroup tree below. On a normal boot stage-1 already
    // mounted them (this is then a no-op); in the initrd we are the first init
    // and must do it ourselves, like upstream systemd.
    mount_api_filesystems();

    // Create the standard /dev symlinks (matching systemd's dev_setup()):
    // /dev/fd, /dev/stdin, /dev/stdout, /dev/stderr and /dev/core. Shell
    // process substitution (`diff <(a) <(b)`) and `/dev/stdin` redirects open
    // /dev/fd/N via these; without them such constructs fail with "No such file
    // or directory" on the global /dev.
    create_standard_dev_symlinks();

    // When running as PID 1, the inherited stdin/stdout/stderr may be broken
    // pipes (e.g. the NixOS stage-2 init script redirects stdout through a
    // tee process that can die before exec'ing the service manager).  Reopen
    // the standard file descriptors matching what real systemd does at startup:
    //   - stdin  → /dev/null   (PID 1 never reads from the console)
    //   - stdout → /dev/console (write-only, O_NOCTTY)
    //   - stderr → /dev/console (write-only, O_NOCTTY)
    //
    // Opening /dev/console without O_NOCTTY would make it PID 1's controlling
    // terminal, which prevents getty from later acquiring it via TIOCSCTTY and
    // breaks terminal control for login shells.  Real systemd uses
    // make_null_stdio() + a separate write-only /dev/console fd for logging.

    // stdin → /dev/null
    let null_fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if null_fd >= 0 {
        let _ = unsafe { libc::dup2(null_fd, libc::STDIN_FILENO) };
        if null_fd > libc::STDERR_FILENO {
            unsafe { libc::close(null_fd) };
        }
    }

    // stdout/stderr → /dev/console (write-only, O_NOCTTY)
    let console_fd = unsafe {
        libc::open(
            c"/dev/console".as_ptr(),
            libc::O_WRONLY | libc::O_NOCTTY | libc::O_CLOEXEC,
        )
    };
    if console_fd >= 0 {
        let _ = unsafe { libc::dup2(console_fd, libc::STDOUT_FILENO) };
        let _ = unsafe { libc::dup2(console_fd, libc::STDERR_FILENO) };
        if console_fd > libc::STDERR_FILENO {
            unsafe { libc::close(console_fd) };
        }
    }

    // Remount root filesystem read-write if needed.
    let flags = nix::mount::MsFlags::MS_REMOUNT;
    let source: Option<&str> = None;
    let fs_type: Option<&str> = None;
    let data: Option<&str> = None;
    let _ = nix::mount::mount(source, "/", fs_type, flags, data);

    // Mount essential tmpfs mount points that NixOS services depend on.
    //
    // Mount units are now implemented, but these early hardcoded mounts
    // serve as fallbacks that run before the unit loading pipeline.
    // Several NixOS services (notably suid-sgid-wrappers.service) have
    // RequiresMountsFor= dependencies on paths that need a tmpfs.  Without
    // these mounts the wrapper setup fails, which breaks PAM/NSS and
    // prevents login.
    //
    // Each entry is (where, options).  We only mount if the path doesn't
    // already have something mounted on it (checked via a simple stat of
    // the path — if the directory doesn't exist we create it first).
    let tmpfs_mounts: &[(&str, nix::mount::MsFlags, &str)] = &[
        (
            "/run/wrappers",
            nix::mount::MsFlags::MS_NODEV,
            "mode=755,size=50%",
        ),
        ("/run/initramfs", nix::mount::MsFlags::empty(), "mode=0700"),
    ];

    for &(where_path, flags, options) in tmpfs_mounts {
        // Create the mount point directory if it doesn't exist
        let _ = std::fs::create_dir_all(where_path);

        // Skip if already mounted (a very rough check: try to mount and
        // ignore EBUSY which means something is already there).
        let mount_result = nix::mount::mount(
            Some("tmpfs"),
            where_path,
            Some("tmpfs"),
            flags,
            Some(options),
        );
        match mount_result {
            Ok(()) => {
                eprintln!("rust-systemd: mounted tmpfs on {where_path}");
            }
            Err(nix::Error::EBUSY) => {
                // Already mounted — fine
            }
            Err(e) => {
                eprintln!("rust-systemd: failed to mount tmpfs on {where_path}: {e}");
            }
        }
    }

    // Mount cgroup2 filesystem at /sys/fs/cgroup.
    //
    // Real systemd mounts this very early during PID 1 initialization.
    // Many services (notably systemd-journald) need cgroups to work —
    // without this mount, journald fails with "Failed to acquire cgroup
    // root path: No medium found" and exits before sending READY=1.
    //
    // We mount cgroup2 (the unified hierarchy) which is what modern
    // systemd expects.  The mount is done with nsdelegate and memory_recursiveprot
    // options matching what real systemd uses.
    let _ = std::fs::create_dir_all("/sys/fs/cgroup");
    match nix::mount::mount(
        Some("cgroup2"),
        "/sys/fs/cgroup",
        Some("cgroup2"),
        nix::mount::MsFlags::MS_NOSUID
            | nix::mount::MsFlags::MS_NODEV
            | nix::mount::MsFlags::MS_NOEXEC,
        Some("nsdelegate,memory_recursiveprot"),
    ) {
        Ok(()) => {
            eprintln!("rust-systemd: mounted cgroup2 on /sys/fs/cgroup");
        }
        Err(nix::Error::EBUSY) => {
            // Already mounted — fine
        }
        Err(e) => {
            eprintln!("rust-systemd: failed to mount cgroup2 on /sys/fs/cgroup: {e}");
        }
    }

    // Bring up the loopback network interface.
    //
    // Real systemd calls loopback_setup() very early during PID 1
    // initialization to ensure localhost networking is available before
    // any services start.  Without this, the loopback interface stays
    // DOWN (the kernel creates it but doesn't activate it), which breaks
    // services that bind to 127.0.0.1 and integration tests that check
    // for LOOPBACK,UP.
    //
    // Skip during daemon-reexec — real systemd only brings up loopback
    // during a full (first) setup, not during re-execution.
    let is_reexec = std::env::var("SYSTEMD_RS_REEXEC").is_ok_and(|v| v == "1");
    if !is_reexec {
        bring_up_loopback();
    }

    // Ensure /etc/machine-id exists.
    //
    // systemd-journald uses the machine-id to name the journal directory
    // under /var/log/journal/<machine-id>/.  If /etc/machine-id is missing
    // or empty, journald cannot create persistent storage and may fail.
    // Real systemd generates this file very early (via systemd-machine-id-setup
    // or first-boot logic).  We generate a random one if it doesn't exist.
    let machine_id_path = std::path::Path::new("/etc/machine-id");
    if !machine_id_path.exists()
        || std::fs::metadata(machine_id_path).map_or(true, |m| m.len() == 0)
    {
        // Generate a random 128-bit ID formatted as 32 hex chars + newline
        let mut buf = [0u8; 16];
        if let Ok(f) = std::fs::File::open("/dev/urandom") {
            use std::io::Read;
            let mut f = f;
            if f.read_exact(&mut buf).is_ok() {
                let hex: String = buf.iter().map(|b| format!("{b:02x}")).collect();
                if std::fs::write(machine_id_path, format!("{hex}\n")).is_ok() {
                    eprintln!("rust-systemd: generated /etc/machine-id");
                }
            }
        }
    }

    // Set the system hostname from /etc/hostname.
    //
    // Real systemd reads /etc/hostname very early during PID 1
    // initialization and calls sethostname(2).  Without this, the
    // kernel hostname remains empty (shown as "(none)" in prompts
    // and login banners).
    let hostname_path = std::path::Path::new("/etc/hostname");
    if hostname_path.exists()
        && let Ok(raw) = std::fs::read_to_string(hostname_path)
    {
        let hostname = raw.trim();
        if !hostname.is_empty() {
            match nix::unistd::sethostname(hostname) {
                Ok(()) => {
                    eprintln!("rust-systemd: set hostname to '{hostname}'");
                }
                Err(e) => {
                    eprintln!("rust-systemd: failed to set hostname: {e}");
                }
            }
        }
    }

    // Ensure home directories exist for all users in /etc/passwd.
    //
    // On NixOS the activation script runs update-users-groups.pl
    // BEFORE exec'ing into rust-systemd, so /etc/passwd is already
    // populated with all declared users by this point.  However the
    // Perl script may fail to create the home directory (e.g. missing
    // /var/lib/nixos state dir on first boot).  We create any missing
    // home directories here — before services (including getty) start —
    // to avoid the "No directory, logging in with HOME=/" message.
    ensure_home_directories();

    // Ensure /var/log/journal exists so that systemd-journald can use
    // persistent storage and `journalctl --flush` succeeds.  Normally
    // systemd-tmpfiles-setup creates this, but it may run after (or
    // concurrently with) systemd-journal-flush.service.  This early
    // creation serves as a fallback before mount units are activated.
    let _ = std::fs::create_dir_all("/var/log/journal");

    // Ensure /var/lib/lastlog exists so that pam_lastlog2 can open/create
    // its SQLite database (lastlog2.db).  Normally systemd-tmpfiles-setup
    // creates this, but getty may start before tmpfiles-setup completes,
    // causing pam_lastlog2 to return PAM_SESSION_ERR and login to print
    // "System error" on the first attempt.
    let _ = std::fs::create_dir_all("/var/lib/lastlog");

    // ── PAM / NSS prerequisite diagnostics ──────────────────────────────
    //
    // Log the state of critical files that PAM and NSS need.  If any of
    // these are missing at this point, authentication will fail later with
    // "Authentication service cannot retrieve authentication info".
    //
    // /etc/shadow  – password hashes; must exist and be readable by root
    // /etc/nsswitch.conf – NSS module configuration
    // /run/wrappers/bin/unix_chkpwd – suid helper used by pam_unix
    //   (created later by suid-sgid-wrappers.service, so it is expected
    //    to be absent here; logged for post-mortem debugging)

    let shadow = std::path::Path::new("/etc/shadow");
    if shadow.exists() {
        eprintln!("rust-systemd: /etc/shadow exists (ok)");
    } else {
        eprintln!(
            "rust-systemd: WARNING: /etc/shadow does not exist — PAM authentication will fail"
        );
    }

    let nsswitch = std::path::Path::new("/etc/nsswitch.conf");
    if nsswitch.exists() {
        eprintln!("rust-systemd: /etc/nsswitch.conf exists (ok)");
    } else {
        eprintln!(
            "rust-systemd: WARNING: /etc/nsswitch.conf does not exist — NSS lookups may fail"
        );
    }

    let chkpwd = std::path::Path::new("/run/wrappers/bin/unix_chkpwd");
    if chkpwd.exists() {
        eprintln!("rust-systemd: /run/wrappers/bin/unix_chkpwd exists (ok)");
    } else {
        eprintln!(
            "rust-systemd: /run/wrappers/bin/unix_chkpwd not yet present \
             (expected — suid-sgid-wrappers.service will create it)"
        );
    }

    // Check that /run/wrappers is mounted without nosuid — the suid bit on
    // unix_chkpwd only works if the filesystem allows it.
    if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
        for line in mounts.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() >= 4 && fields[1] == "/run/wrappers" {
                let opts = fields[3];
                if opts.split(',').any(|o| o == "nosuid") {
                    eprintln!(
                        "rust-systemd: WARNING: /run/wrappers is mounted with nosuid — \
                         suid wrappers will not work! (options: {opts})"
                    );
                } else {
                    eprintln!(
                        "rust-systemd: /run/wrappers mounted without nosuid (ok, options: {opts})"
                    );
                }
                break;
            }
        }
    }
}

/// Bring up the loopback network interface in the host network namespace.
///
/// Uses ioctl to set the IFF_UP flag on the "lo" interface, matching what
/// real systemd does via `loopback_setup()` during early PID 1 init.
fn bring_up_loopback() {
    let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if sock < 0 {
        eprintln!(
            "rust-systemd: failed to create socket for loopback setup: {}",
            std::io::Error::last_os_error()
        );
        return;
    }

    let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
    let lo_name = b"lo\0";
    unsafe {
        std::ptr::copy_nonoverlapping(
            lo_name.as_ptr(),
            ifr.ifr_name.as_mut_ptr().cast(),
            lo_name.len(),
        );
    }

    // Get current flags
    let ret = unsafe { libc::ioctl(sock, libc::SIOCGIFFLAGS, &mut ifr) };
    if ret != 0 {
        eprintln!(
            "rust-systemd: failed to get loopback flags: {}",
            std::io::Error::last_os_error()
        );
        unsafe { libc::close(sock) };
        return;
    }

    // Set IFF_UP
    unsafe {
        ifr.ifr_ifru.ifru_flags |= libc::IFF_UP as libc::c_short;
    }
    let ret = unsafe { libc::ioctl(sock, libc::SIOCSIFFLAGS, &ifr) };
    if ret == 0 {
        eprintln!("rust-systemd: brought up loopback interface");
    } else {
        eprintln!(
            "rust-systemd: failed to bring up loopback interface: {}",
            std::io::Error::last_os_error()
        );
    }

    unsafe { libc::close(sock) };
}

/// Read /etc/passwd and create any missing home directories with the
/// correct ownership and mode.  Skips trivial homes like "/" and
/// "/var/empty".
fn ensure_home_directories() {
    let passwd_path = std::path::Path::new("/etc/passwd");
    if !passwd_path.exists() {
        eprintln!("rust-systemd: /etc/passwd does not exist, skipping home directory creation");
        return;
    }
    let passwd_contents = match std::fs::read_to_string(passwd_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("rust-systemd: failed to read /etc/passwd: {e}");
            return;
        }
    };

    let line_count = passwd_contents.lines().count();
    eprintln!("rust-systemd: ensuring home directories ({line_count} passwd entries)");

    for line in passwd_contents.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        // passwd format: name:x:uid:gid:gecos:home:shell
        if fields.len() < 7 {
            continue;
        }
        let user = fields[0];
        let home = fields[5];
        // Skip non-directory or special homes
        if home.is_empty()
            || home == "/"
            || home == "/nonexistent"
            || home == "/dev/null"
            || home == "/var/empty"
            || home == "/run/systemd"
        {
            continue;
        }
        let home_path = std::path::Path::new(home);
        if home_path.exists() {
            continue;
        }
        let uid: u32 = match fields[2].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let gid: u32 = match fields[3].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        match std::fs::create_dir_all(home_path) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("rust-systemd: failed to create home {home} for {user}: {e}");
                continue;
            }
        }
        // Set ownership to the user/group from passwd
        let c_path = match std::ffi::CString::new(home) {
            Ok(p) => p,
            Err(_) => continue,
        };
        unsafe { libc::chown(c_path.as_ptr(), uid, gid) };
        unsafe { libc::chmod(c_path.as_ptr(), 0o755) };
        eprintln!("rust-systemd: created home directory {home} for {user} (uid={uid}, gid={gid})");
    }
}

#[cfg(not(target_os = "linux"))]
fn pid1_specific_setup() {}

fn prepare_runtimeinfo(conf: &config::Config, dry_run: bool) -> runtime_info::ArcMutRuntimeInfo {
    // initial loading of the units and matching of the various before/after settings
    // also opening all fildescriptors in the socket files
    //
    // During daemon-reexec, skip pruning so that transient units (e.g. from
    // systemd-run) that are not reachable from the boot target are still loaded.
    let is_reexec = std::env::var("SYSTEMD_RS_REEXEC").is_ok_and(|v| v == "1");
    let mut unit_table = if is_reexec {
        units::load_all_units_no_prune(&conf.unit_dirs, &conf.target_unit)
    } else {
        units::load_all_units(&conf.unit_dirs, &conf.target_unit)
    }
    .expect("loading unit files");
    trace!("Finished loading units");

    // Break dependency cycles instead of aborting, matching systemd behavior.
    // systemd warns about cycles and removes ordering edges to break them.
    let broken_cycles = units::break_dependency_cycles(&mut unit_table);
    // Boot-time cycle IDs are recorded after RuntimeInfo is created (see below).
    let boot_cycle_count = broken_cycles.len();
    if boot_cycle_count > 0 {
        warn!(
            "Broke {} dependency cycle(s). See warnings above for details.",
            boot_cycle_count
        );
    }
    trace!("Unit dependencies passed sanity checks");
    let unit_table = unit_table;

    if dry_run {
        warn!("Exit after loading because --dry-run was passed");
        unrecoverable_error("Started as dry-run".into());
    }

    let pid_table = Arc::new(Mutex::new(std::collections::HashMap::new()));

    Arc::new(RwLock::new(runtime_info::RuntimeInfo {
        unit_table,
        pid_table,
        fd_store: std::sync::RwLock::new(crate::fd_store::FDStore::default()),
        config: conf.clone(),
        stdout_eventfd: platform::make_event_fd().unwrap(),
        stderr_eventfd: platform::make_event_fd().unwrap(),
        notification_eventfd: platform::make_event_fd().unwrap(),
        socket_activation_eventfd: platform::make_event_fd().unwrap(),
        pending_activations: std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::HashSet::new(),
        )),
        manager_environment: {
            let mut env = std::collections::HashMap::new();
            for (k, v) in std::env::vars() {
                env.insert(k, v);
            }
            std::sync::Arc::new(std::sync::Mutex::new(env))
        },
        unit_markers: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        transactions_with_cycle: {
            let mut ids = Vec::new();
            for cycle in &broken_cycles {
                let txn_id = crate::control::varlink::next_transaction_id();
                ids.push(txn_id);
                let cycle_names: Vec<String> = cycle.iter().map(|id| id.name.clone()).collect();
                let msg = format!(
                    "Found ordering cycle starting with {}",
                    cycle_names.join(" -> ")
                );
                let txn_str = txn_id.to_string();
                crate::control::varlink::journal_log_with_fields(
                    &msg,
                    4,
                    &[("TRANSACTION_ID", &txn_str)],
                );
            }
            std::sync::Arc::new(std::sync::Mutex::new(ids))
        },
        units_in_cycles: {
            let mut cycle_units = std::collections::HashSet::new();
            for cycle in &broken_cycles {
                for id in cycle {
                    cycle_units.insert(id.name.clone());
                }
            }
            std::sync::Arc::new(std::sync::Mutex::new(cycle_units))
        },
    }))
}

fn start_notification_handler_thread(run_info: runtime_info::ArcMutRuntimeInfo) {
    std::thread::spawn(move || {
        notification_handler::handle_all_streams(run_info);
    });
}
fn start_stdout_handler_thread(run_info: runtime_info::ArcMutRuntimeInfo) {
    std::thread::spawn(move || {
        notification_handler::handle_all_std_out(run_info);
    });
}
fn start_stderr_handler_thread(run_info: runtime_info::ArcMutRuntimeInfo) {
    std::thread::spawn(move || {
        notification_handler::handle_all_std_err(run_info);
    });
}
fn start_signal_handler_thread(
    signals: Signals,
    run_info: runtime_info::ArcMutRuntimeInfo,
) -> std::thread::JoinHandle<()> {
    // Clone the pid_table Arc *before* moving run_info into the closure.
    // The signal handler uses this separate handle to update PID entries
    // (Service → ServiceExited) without acquiring the RuntimeInfo read lock,
    // breaking the 3-way deadlock described in signal_handler.rs.
    let pid_table = run_info.read_poisoned().pid_table.clone();
    std::thread::spawn(move || {
        // listen on signals from the child processes
        signal_handler::handle_signals(signals, run_info, pid_table);
    })
}

use clap::Parser;

#[derive(Parser, Debug)]
struct CliArgs {
    #[arg(short, long)]
    dry_run: bool,
}

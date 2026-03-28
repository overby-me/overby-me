use log::error;
use log::trace;
use log::warn;
use which::which;

use super::fork_child;
use crate::fd_store::FDStore;
use crate::services::RunCmdError;
use crate::services::Service;
use crate::units::{CommandlinePrefix, ServiceConfig, StdIoOption};

use std::path::Path;

/// Resolve a User= value (name or numeric UID) to a raw `uid_t`.
/// Falls back to the current process UID if `user` is `None`.
pub fn resolve_uid(user: &Option<String>) -> Result<libc::uid_t, String> {
    match user {
        Some(user_str) => {
            if let Ok(uid) = user_str.parse::<u32>() {
                Ok(uid)
            } else {
                let pwentry = crate::platform::pwnam::getpwnam_r(user_str)
                    .map_err(|_| format!("Couldn't resolve uid for username: {user_str}"))?;
                Ok(pwentry.uid.as_raw())
            }
        }
        None => Ok(nix::unistd::getuid().as_raw()),
    }
}

/// Resolve a Group= value (name or numeric GID) to a raw `gid_t`.
/// Falls back to the current process GID if `group` is `None`.
pub fn resolve_gid(group: &Option<String>) -> Result<libc::gid_t, String> {
    match group {
        Some(group_str) => {
            if let Ok(gid) = group_str.parse::<u32>() {
                Ok(gid)
            } else {
                let grentry = crate::platform::grnam::getgrnam_r(group_str)
                    .map_err(|_| format!("Couldn't resolve gid for groupname: {group_str}"))?;
                Ok(grentry.gid.as_raw())
            }
        }
        None => Ok(nix::unistd::getgid().as_raw()),
    }
}

/// Resolve a Group= value, falling back to the User='s primary group if
/// `group` is `None` but `user` is `Some`. This matches systemd behavior
/// where setting User= without Group= runs the service with the user's
/// primary group.
fn resolve_gid_with_user_fallback(
    group: &Option<String>,
    user: &Option<String>,
) -> Result<libc::gid_t, String> {
    if group.is_some() {
        return resolve_gid(group);
    }
    if let Some(user_str) = user {
        // Look up the user's primary group
        if user_str.parse::<u32>().is_err() {
            let pwentry = crate::platform::pwnam::getpwnam_r(user_str)
                .map_err(|_| format!("Couldn't resolve user for group fallback: {user_str}"))?;
            return Ok(pwentry.gid.as_raw());
        }
    }
    resolve_gid(group)
}

/// Resolve a list of SupplementaryGroups= values to raw `gid_t` values.
fn resolve_supplementary_gids(groups: &[String]) -> Result<Vec<libc::gid_t>, String> {
    groups
        .iter()
        .map(|g| {
            if let Ok(gid) = g.parse::<u32>() {
                Ok(gid)
            } else {
                let grentry = crate::platform::grnam::getgrnam_r(g)
                    .map_err(|_| format!("Couldn't resolve gid for supplementary group: {g}"))?;
                Ok(grentry.gid.as_raw())
            }
        })
        .collect()
}

fn start_service_with_filedescriptors(
    self_path: &Path,
    srvc: &mut Service,
    conf: &ServiceConfig,
    name: &str,
    fd_store: &FDStore,
) -> Result<(), RunCmdError> {
    let exec = conf.exec.last().ok_or_else(|| {
        RunCmdError::SpawnError(
            name.to_owned(),
            "Service has no ExecStart command".to_string(),
        )
    })?;

    // check if executable even exists
    let cmd = which(&exec.cmd).map_err(|err| {
        RunCmdError::SpawnError(
            name.to_owned(),
            format!("Could not resolve command to an executable file: {err:?}"),
        )
    })?;
    if !cmd.exists() {
        error!(
            "The service {} specified an executable that does not exist: {:?}",
            name, &exec.cmd
        );
        return Err(RunCmdError::SpawnError(
            exec.cmd.clone(),
            "Executable does not exist".to_string(),
        ));
    }
    if !cmd.is_file() {
        error!(
            "The service {} specified an executable that is not a file: {:?}",
            name, &cmd
        );
        return Err(RunCmdError::SpawnError(
            exec.cmd.clone(),
            "Executable does not exist (is a directory)".to_string(),
        ));
    }

    // 1. fork
    // 1. in fork use dup2 to map all relevant file desrciptors to 3..x
    // 1. in fork mark all other file descriptors with FD_CLOEXEC
    // 1. in fork set relevant env variables $LISTEN_FDS $LISTEN_PID
    // 1. in fork execve the cmd with the args
    // 1. in parent set pid and return. Waiting will be done afterwards if necessary

    let notifications_path = {
        if let Some(p) = &srvc.notifications_path {
            p.to_str().unwrap().to_owned()
        } else {
            return Err(RunCmdError::Generic(format!(
                "Tried to start service: {name} without a notifications path",
            )));
        }
    };

    super::fork_os_specific::pre_fork_os_specific(conf).map_err(RunCmdError::Generic)?;

    let mut fds = Vec::new();
    let mut names = Vec::new();

    for socket in &conf.sockets {
        // Skip sockets whose FDs aren't in the store — this can happen when
        // a socket unit's conditions failed (e.g. ConditionSecurity=audit on
        // systemd-journald-audit.socket) and the socket was never opened.
        let Some(global_fds) = fd_store.get_global(&socket.name) else {
            log::trace!(
                "Socket {} has no FDs in store (condition-skipped?), skipping for service {name}",
                socket.name
            );
            continue;
        };

        let sock_fds = global_fds
            .iter()
            .map(|(_, _, fd)| fd.as_raw_fd())
            .collect::<Vec<_>>();

        let sock_names = global_fds
            .iter()
            .map(|(_, name, _)| name.clone())
            .collect::<Vec<_>>();

        fds.extend(sock_fds);
        names.extend(sock_names);
    }

    // Also pass any file descriptors stored via FDSTORE=1 sd_notify messages.
    // These are FDs the service asked PID 1 to hold across restarts.
    // They are appended after socket-activation FDs, matching real systemd
    // behavior where sd_listen_fds() returns socket FDs first, then stored FDs.
    if !srvc.stored_fds.is_empty() {
        trace!(
            "Service {name}: passing {} stored fd(s) from FDSTORE",
            srvc.stored_fds.len()
        );
        for (fd_name, raw_fd) in &srvc.stored_fds {
            fds.push(*raw_fd);
            names.push(fd_name.clone());
        }
    }

    // We first exec into our own executable again and apply this config
    // We transfer the config via a anonymous shared memory file
    let exec_helper_conf = crate::entrypoints::ExecHelperConfig {
        name: name.to_owned(),
        // Pass the manager's current log level to the exec helper, mirroring
        // real systemd's `--log-level` argument to sd-executor.  The exec
        // helper uses this as the default for its KmsgLogger; the unit's
        // SYSTEMD_LOG_LEVEL env var (if set) takes final precedence.
        log_level: Some(
            match log::max_level() {
                log::LevelFilter::Off => "error",
                log::LevelFilter::Error => "error",
                log::LevelFilter::Warn => "warn",
                log::LevelFilter::Info => "info",
                log::LevelFilter::Debug => "debug",
                log::LevelFilter::Trace => "trace",
            }
            .to_owned(),
        ),
        cmd,
        args: exec.args.clone(),
        use_first_arg_as_argv0: exec.prefixes.contains(&CommandlinePrefix::AtSign),
        privileged_prefix: exec.prefixes.contains(&CommandlinePrefix::Plus)
            || exec.prefixes.contains(&CommandlinePrefix::Exclamation)
            || exec
                .prefixes
                .contains(&CommandlinePrefix::DoubleExclamation),
        clean_environment: exec.prefixes.contains(&CommandlinePrefix::Colon),
        env: {
            let default_path = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
            let mut env = vec![
                // Use the inherited PATH if available (important for NixOS where
                // executables live in /nix/store and are reachable via
                // /run/current-system/sw/bin, /run/wrappers/bin, etc.).
                // Fall back to the systemd default FHS PATH (see man systemd.exec).
                (
                    "PATH".to_owned(),
                    std::env::var("PATH").unwrap_or_else(|_| default_path.to_owned()),
                ),
            ];
            // Set HOME, USER, LOGNAME, SHELL from the User= setting.
            // systemd populates these automatically when User= is set.
            if let Some(ref user_str) = conf.exec_config.user
                && let Ok(pwentry) = crate::platform::pwnam::getpwnam_r(user_str)
            {
                env.push(("USER".to_owned(), user_str.clone()));
                env.push(("LOGNAME".to_owned(), user_str.clone()));
                env.push(("HOME".to_owned(), pwentry.home.clone()));
                env.push(("SHELL".to_owned(), pwentry.shell.clone()));
            }
            // Load EnvironmentFile= files first (lower priority than Environment=).
            // Each file contains lines of KEY=VALUE pairs.
            for (path, optional) in &conf.exec_config.environment_files {
                match std::fs::read_to_string(path) {
                    Ok(contents) => {
                        for line in contents.lines() {
                            let line = line.trim();
                            // Skip comments and empty lines
                            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                                continue;
                            }
                            if let Some((key, value)) = line.split_once('=') {
                                let key = key.trim().to_owned();
                                // Strip optional surrounding quotes from the value
                                let value = value.trim();
                                let value = if (value.starts_with('"') && value.ends_with('"'))
                                    || (value.starts_with('\'') && value.ends_with('\''))
                                {
                                    value[1..value.len() - 1].to_owned()
                                } else {
                                    value.to_owned()
                                };
                                env.retain(|(ek, _)| *ek != key);
                                env.push((key, value));
                            }
                        }
                    }
                    Err(e) => {
                        if *optional {
                            trace!(
                                "Optional EnvironmentFile not found for {}: {:?}: {}",
                                name, path, e
                            );
                        } else {
                            warn!(
                                "Failed to read EnvironmentFile for {}: {:?}: {}",
                                name, path, e
                            );
                        }
                    }
                }
            }
            // Apply the service's Environment= settings (may override PATH
            // and EnvironmentFile= values)
            if let Some(ref vars) = conf.exec_config.environment {
                for (k, v) in &vars.vars {
                    // Remove any prior entry with the same key so the unit's
                    // value takes precedence over the defaults above.
                    env.retain(|(ek, _)| ek != k);
                    env.push((k.clone(), v.clone()));
                }
            }
            // PassEnvironment= — import listed variable names from the system
            // manager's (PID 1) environment. Variables that are not set in the
            // manager's environment are silently ignored. Applied after
            // Environment= so they can override unit-level settings, but
            // before internal variables and UnsetEnvironment=.
            for var_name in &conf.exec_config.pass_environment {
                if let Ok(value) = std::env::var(var_name) {
                    env.retain(|(ek, _)| ek != var_name);
                    env.push((var_name.clone(), value));
                }
            }
            // Internal variables added last — these must not be overridden.
            // Only set LISTEN_FDS/LISTEN_FDNAMES when there are actual FDs to
            // pass. Real systemd does not set these for services without
            // sockets, and some services (e.g. systemd-logind) call
            // sd_listen_fds_with_names() which returns -EINVAL when
            // LISTEN_FDNAMES is present but mismatches LISTEN_FDS.
            if !names.is_empty() {
                env.push(("LISTEN_FDS".to_owned(), format!("{}", names.len())));
                env.push(("LISTEN_FDNAMES".to_owned(), names.join(":")));
            }
            env.push(("NOTIFY_SOCKET".to_owned(), notifications_path));

            // INVOCATION_ID — a unique 128-bit identifier for each service
            // invocation, formatted as lowercase hex without dashes.
            {
                use std::io::Read;
                let mut buf = [0u8; 16];
                if let Ok(mut f) = std::fs::File::open("/dev/urandom")
                    && f.read_exact(&mut buf).is_ok()
                {
                    // Set version 4 (random) UUID variant bits
                    buf[6] = (buf[6] & 0x0f) | 0x40;
                    buf[8] = (buf[8] & 0x3f) | 0x80;
                    let id = buf.iter().map(|b| format!("{b:02x}")).collect::<String>();
                    srvc.invocation_id = Some(id.clone());
                    env.push(("INVOCATION_ID".to_owned(), id));
                }
            }

            // TRIGGER_PATH / TRIGGER_UNIT — set by path watcher when a .path
            // unit triggers this service. Cleared after use (one-shot).
            if let Some(ref tp) = srvc.trigger_path {
                env.push(("TRIGGER_PATH".to_owned(), tp.clone()));
            }
            if let Some(ref tu) = srvc.trigger_unit {
                env.push(("TRIGGER_UNIT".to_owned(), tu.clone()));
            }
            if let Some(usec) = srvc.trigger_timer_realtime_usec {
                env.push(("TRIGGER_TIMER_REALTIME_USEC".to_owned(), usec.to_string()));
            }
            if let Some(usec) = srvc.trigger_timer_monotonic_usec {
                env.push(("TRIGGER_TIMER_MONOTONIC_USEC".to_owned(), usec.to_string()));
            }

            // MONITOR_* env vars for OnSuccess=/OnFailure= handler services.
            if let Some(ref mon) = srvc.monitor_env {
                env.push((
                    "MONITOR_SERVICE_RESULT".to_owned(),
                    mon.service_result.clone(),
                ));
                env.push(("MONITOR_EXIT_CODE".to_owned(), mon.exit_code.clone()));
                env.push(("MONITOR_EXIT_STATUS".to_owned(), mon.exit_status.clone()));
                env.push(("MONITOR_UNIT".to_owned(), mon.unit.clone()));
                env.push(("MONITOR_INVOCATION_ID".to_owned(), "0".to_owned()));
            }

            // UnsetEnvironment= is applied as the final step (see systemd.exec(5)).
            // It can undo assignments from any source, including Environment=,
            // EnvironmentFile=, PassEnvironment=, and even internal variables.
            // If an entry contains '=' it is a VAR=VALUE match (only exact match
            // is removed). Otherwise it is a plain variable name and any
            // assignment with that name is removed regardless of value.
            for entry in &conf.exec_config.unset_environment {
                if let Some((key, value)) = entry.split_once('=') {
                    // Remove only the exact VAR=VALUE match
                    env.retain(|(ek, ev)| !(ek == key && ev == value));
                } else {
                    // Remove any assignment with this variable name
                    env.retain(|(ek, _)| ek != entry);
                }
            }

            env
        },
        group: resolve_gid_with_user_fallback(&conf.exec_config.group, &conf.exec_config.user)
            .map_err(|e| RunCmdError::SpawnError(name.to_owned(), e))?,
        supplementary_groups: resolve_supplementary_gids(&conf.exec_config.supplementary_groups)
            .map_err(|e| RunCmdError::SpawnError(name.to_owned(), e))?,
        user: resolve_uid(&conf.exec_config.user)
            .map_err(|e| RunCmdError::SpawnError(name.to_owned(), e))?,

        working_directory: conf.exec_config.working_directory.clone(),
        state_directory: conf.exec_config.state_directory.clone(),
        logs_directory: conf.exec_config.logs_directory.clone(),
        logs_directory_mode: conf.exec_config.logs_directory_mode,
        runtime_directory: conf.exec_config.runtime_directory.clone(),

        oom_score_adjust: conf.exec_config.oom_score_adjust,

        platform_specific: conf.platform_specific.clone(),

        limit_nofile: conf.limit_nofile,

        stdin_option: conf.exec_config.stdin_option.clone(),
        tty_path: conf.exec_config.tty_path.clone(),
        tty_reset: conf.exec_config.tty_reset,
        tty_vhangup: conf.exec_config.tty_vhangup,
        tty_vt_disallocate: conf.exec_config.tty_vt_disallocate,
        ignore_sigpipe: conf.exec_config.ignore_sigpipe,
        utmp_identifier: conf.exec_config.utmp_identifier.clone(),
        utmp_mode: conf.exec_config.utmp_mode,
        import_credentials: conf.exec_config.import_credentials.clone(),
        load_credentials: conf.exec_config.load_credentials.clone(),
        load_credentials_encrypted: conf.exec_config.load_credentials_encrypted.clone(),
        set_credentials: conf.exec_config.set_credentials.clone(),
        set_credentials_encrypted: conf.exec_config.set_credentials_encrypted.clone(),
        stdout_is_inherit: matches!(
            conf.exec_config.stdout_path,
            None | Some(StdIoOption::Inherit)
                | Some(StdIoOption::Journal)
                | Some(StdIoOption::Kmsg)
                | Some(StdIoOption::Tty)
        ),
        stderr_is_inherit: matches!(
            conf.exec_config.stderr_path,
            None | Some(StdIoOption::Inherit)
                | Some(StdIoOption::Journal)
                | Some(StdIoOption::Kmsg)
                | Some(StdIoOption::Tty)
        ),
        stdout_is_tty: matches!(conf.exec_config.stdout_path, Some(StdIoOption::Tty)),
        stderr_is_tty: matches!(conf.exec_config.stderr_path, Some(StdIoOption::Tty)),
        stdout_file_path: match &conf.exec_config.stdout_path {
            Some(StdIoOption::File(p)) | Some(StdIoOption::AppendFile(p)) => {
                Some(p.to_string_lossy().into_owned())
            }
            _ => None,
        },
        stdout_file_append: matches!(
            conf.exec_config.stdout_path,
            Some(StdIoOption::AppendFile(_))
        ),
        stderr_file_path: match &conf.exec_config.stderr_path {
            Some(StdIoOption::File(p)) | Some(StdIoOption::AppendFile(p)) => {
                Some(p.to_string_lossy().into_owned())
            }
            _ => None,
        },
        stderr_file_append: matches!(
            conf.exec_config.stderr_path,
            Some(StdIoOption::AppendFile(_))
        ),
        ambient_capabilities: conf.exec_config.ambient_capabilities.clone(),

        // Security & sandboxing directives
        no_new_privileges: conf.exec_config.no_new_privileges,
        umask: conf.exec_config.umask,
        nice: conf.exec_config.nice,
        cpu_scheduling_policy: conf.exec_config.cpu_scheduling_policy.clone(),
        cpu_scheduling_priority: conf.exec_config.cpu_scheduling_priority,
        cpu_scheduling_reset_on_fork: conf.exec_config.cpu_scheduling_reset_on_fork,
        io_scheduling_class: match conf.exec_config.io_scheduling_class {
            crate::units::IOSchedulingClass::None => 0,
            crate::units::IOSchedulingClass::Realtime => 1,
            crate::units::IOSchedulingClass::BestEffort => 2,
            crate::units::IOSchedulingClass::Idle => 3,
        },
        io_scheduling_priority: conf.exec_config.io_scheduling_priority,
        capability_bounding_set: conf.exec_config.capability_bounding_set.clone(),
        protect_system: match conf.exec_config.protect_system {
            crate::units::ProtectSystem::No => "no".to_owned(),
            crate::units::ProtectSystem::Yes => "yes".to_owned(),
            crate::units::ProtectSystem::Full => "full".to_owned(),
            crate::units::ProtectSystem::Strict => "strict".to_owned(),
        },
        protect_home: match conf.exec_config.protect_home {
            crate::units::ProtectHome::No => "no".to_owned(),
            crate::units::ProtectHome::Yes => "yes".to_owned(),
            crate::units::ProtectHome::ReadOnly => "read-only".to_owned(),
            crate::units::ProtectHome::Tmpfs => "tmpfs".to_owned(),
        },
        private_tmp: conf.exec_config.private_tmp,
        private_devices: conf.exec_config.private_devices,
        private_network: conf.exec_config.private_network,
        private_users: conf.exec_config.private_users,
        private_mounts: conf.exec_config.private_mounts,
        private_ipc: conf.exec_config.private_ipc.unwrap_or(false),
        network_namespace_path: conf.exec_config.network_namespace_path.clone(),
        ipc_namespace_path: conf.exec_config.ipc_namespace_path.clone(),
        timer_slack_nsec: conf
            .exec_config
            .timer_slack_nsec
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok()),
        coredump_filter: conf.exec_config.coredump_filter.clone(),
        cpu_affinity: conf.exec_config.cpu_affinity.clone(),
        private_pids: conf.exec_config.private_pids.unwrap_or(false),
        protect_kernel_tunables: conf.exec_config.protect_kernel_tunables,
        protect_kernel_modules: conf.exec_config.protect_kernel_modules,
        protect_kernel_logs: conf.exec_config.protect_kernel_logs,
        protect_control_groups: conf.exec_config.protect_control_groups,
        protect_clock: conf.exec_config.protect_clock,
        protect_hostname: conf.exec_config.protect_hostname,
        lock_personality: conf.exec_config.lock_personality,
        memory_deny_write_execute: conf.exec_config.memory_deny_write_execute,
        restrict_realtime: conf.exec_config.restrict_realtime,
        restrict_suid_sgid: conf.exec_config.restrict_suid_sgid,
        read_write_paths: conf.exec_config.read_write_paths.clone(),
        restrict_namespaces: match conf.exec_config.restrict_namespaces {
            crate::units::RestrictNamespaces::No => "no".to_owned(),
            crate::units::RestrictNamespaces::Yes => "yes".to_owned(),
            crate::units::RestrictNamespaces::Allow(ref v) => v.join(" "),
            crate::units::RestrictNamespaces::Deny(ref v) => format!("~{}", v.join(" ")),
        },
        system_call_architectures: conf.exec_config.system_call_architectures.clone(),
        system_call_filter: conf.exec_config.system_call_filter.clone(),
        system_call_log: conf.exec_config.system_call_log.clone(),
        restrict_file_systems: conf.exec_config.restrict_file_systems.clone(),
        protect_proc: match conf.exec_config.protect_proc {
            crate::units::ProtectProc::Default => "default".to_owned(),
            crate::units::ProtectProc::Noaccess => "noaccess".to_owned(),
            crate::units::ProtectProc::Invisible => "invisible".to_owned(),
            crate::units::ProtectProc::Ptraceable => "ptraceable".to_owned(),
        },
        proc_subset: match conf.exec_config.proc_subset {
            crate::units::ProcSubset::All => "all".to_owned(),
            crate::units::ProcSubset::Pid => "pid".to_owned(),
        },

        // Resource limits
        limit_cpu: conf.exec_config.limit_cpu,
        limit_core: conf.exec_config.limit_core,
        limit_fsize: conf.exec_config.limit_fsize,
        limit_data: conf.exec_config.limit_data,
        limit_stack: conf.exec_config.limit_stack,
        limit_rss: conf.exec_config.limit_rss,
        limit_nproc: conf.exec_config.limit_nproc,
        limit_memlock: conf.exec_config.limit_memlock,
        limit_as: conf.exec_config.limit_as,
        limit_locks: conf.exec_config.limit_locks,
        limit_sigpending: conf.exec_config.limit_sigpending,
        limit_msgqueue: conf.exec_config.limit_msgqueue,
        limit_nice: conf.exec_config.limit_nice,
        limit_rtprio: conf.exec_config.limit_rtprio,
        limit_rttime: conf.exec_config.limit_rttime,

        // Directory management
        cache_directory: conf.exec_config.cache_directory.clone(),
        cache_directory_mode: conf.exec_config.cache_directory_mode,
        configuration_directory: conf.exec_config.configuration_directory.clone(),
        configuration_directory_mode: conf.exec_config.configuration_directory_mode,
        state_directory_mode: conf.exec_config.state_directory_mode,
        runtime_directory_mode: conf.exec_config.runtime_directory_mode,

        // Path-based mount namespace directives
        read_only_paths: conf.exec_config.read_only_paths.clone(),
        inaccessible_paths: conf.exec_config.inaccessible_paths.clone(),
        bind_paths: conf.exec_config.bind_paths.clone(),
        bind_read_only_paths: conf.exec_config.bind_read_only_paths.clone(),
        temporary_file_system: conf.exec_config.temporary_file_system.clone(),

        // Logging directives
        syslog_identifier: conf.exec_config.syslog_identifier.clone(),
    };

    let marshalled_config = serde_json::to_string(&exec_helper_conf).unwrap();

    // crate the shared memory file
    let exec_helper_conf_fd = shmemfdrs::create_shmem(
        std::ffi::CString::new(name).unwrap(),
        marshalled_config.len() + 1,
    );
    if exec_helper_conf_fd < 0 {
        return Err(RunCmdError::CreatingShmemFailed(
            name.to_owned(),
            std::io::Error::from_raw_os_error(exec_helper_conf_fd).kind(),
        ));
    }
    use std::os::unix::io::FromRawFd;
    let mut exec_helper_conf_file = unsafe { std::fs::File::from_raw_fd(exec_helper_conf_fd) };

    // write the config to the file
    use std::io::Write;
    exec_helper_conf_file
        .write_all(marshalled_config.as_bytes())
        .unwrap();
    exec_helper_conf_file.write_all(b"\n").unwrap();
    use std::io::Seek;
    exec_helper_conf_file
        .seek(std::io::SeekFrom::Start(0))
        .unwrap();

    // need to allocate this before forking. Currently this is just static info, we could only do this once...
    let self_path_cstr = std::ffi::CString::new(self_path.to_str().unwrap()).unwrap();
    let name_arg = std::ffi::CString::new("exec_helper").unwrap();
    let self_args = [name_arg.as_ptr(), std::ptr::null()];

    trace!(
        "Start main executable for service: {name}: {:?} {:?}",
        exec_helper_conf.cmd, exec_helper_conf.args
    );
    // When PrivatePIDs= is set, use clone(CLONE_NEWPID) so the child is
    // PID 1 in a new PID namespace from the start — no extra fork needed
    // in exec_helper.
    let fork_result = if exec_helper_conf.private_pids {
        let pid = unsafe {
            libc::syscall(
                libc::SYS_clone,
                (libc::SIGCHLD | libc::CLONE_NEWPID) as libc::c_ulong,
                std::ptr::null::<libc::c_void>(),
                std::ptr::null::<libc::c_int>(),
                std::ptr::null::<libc::c_int>(),
                0 as libc::c_ulong,
            )
        };
        if pid < 0 {
            Err(nix::errno::Errno::last())
        } else if pid == 0 {
            Ok(nix::unistd::ForkResult::Child)
        } else {
            Ok(nix::unistd::ForkResult::Parent {
                child: nix::unistd::Pid::from_raw(pid as i32),
            })
        }
    } else {
        unsafe { nix::unistd::fork() }
    };
    match fork_result {
        Ok(nix::unistd::ForkResult::Parent { child, .. }) => {
            // make sure the file exists until after we fork before closing it
            drop(exec_helper_conf_file);
            srvc.pid = Some(child);
            srvc.process_group = Some(nix::unistd::Pid::from_raw(-child.as_raw()));
            let now = crate::units::UnitTimestamps::now_usec();
            srvc.exec_main_start_timestamp = Some(now);
            srvc.exec_main_handoff_timestamp = Some(now);
        }
        Ok(nix::unistd::ForkResult::Child) => {
            let stdout = {
                if let Some(stdio) = &srvc.stdout {
                    stdio.write_fd()
                } else {
                    unreachable!();
                }
            };
            let stderr = {
                if let Some(stdio) = &srvc.stderr {
                    stdio.write_fd()
                } else {
                    unreachable!();
                }
            };

            fork_child::after_fork_child(
                &self_path_cstr,
                self_args.as_slice(),
                &mut fds,
                stdout,
                stderr,
                exec_helper_conf_fd,
            );
        }
        Err(e) => error!("Fork for service: {name} failed with: {e}"),
    }
    Ok(())
}

pub fn start_service(
    self_path: &Path,
    srvc: &mut Service,
    conf: &ServiceConfig,
    name: &str,
    fd_store: &FDStore,
) -> Result<(), super::RunCmdError> {
    start_service_with_filedescriptors(self_path, srvc, conf, name, fd_store)?;
    Ok(())
}

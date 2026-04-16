use crate::services::Service;
use crate::sockets::Socket;
use crate::units::{
    Common, CommonState, Dependencies, DeviceConfig, DeviceSpecific, DeviceState, ExecConfig,
    MountConfig, MountSpecific, MountState, ParsedCommonConfig, ParsedDeviceConfig,
    ParsedExecSection, ParsedInstallSection, ParsedMountConfig, ParsedPathConfig,
    ParsedServiceConfig, ParsedSingleSocketConfig, ParsedSliceConfig, ParsedSocketConfig,
    ParsedSwapConfig, ParsedTargetConfig, ParsedTimerConfig, ParsedUnitSection, PathCondition,
    PathConfig, PathSpecific, PathState, PlatformSpecificServiceFields, ServiceConfig,
    ServiceSpecific, ServiceState, SingleSocketConfig, SliceConfig, SliceSpecific, SliceState,
    SocketConfig, SocketResult, SocketSpecific, SocketState, Specific, SwapConfig, SwapSpecific,
    SwapState, TargetSpecific, TargetState, TimerConfig, TimerSpecific, TimerState, Unit,
    UnitConfig, UnitId, UnitIdKind, UnitStatus, UnitTimestamps,
};

use log::trace;

use std::convert::TryInto;
use std::path::PathBuf;
use std::sync::RwLock;

/// Build the cgroup directory path for a slice name relative to a root.
///
/// Systemd slice names encode a hierarchy via dashes:
/// - `"testslice.slice"` → `{root}/testslice.slice/`
/// - `"a-b-c.slice"` → `{root}/a.slice/a-b.slice/a-b-c.slice/`
/// - `"-.slice"` (root slice) → `{root}/`
pub fn slice_cgroup_path(root: &std::path::Path, slice_name: &str) -> PathBuf {
    if slice_name == "-.slice" {
        return root.to_path_buf();
    }
    let base = slice_name.strip_suffix(".slice").unwrap_or(slice_name);
    let parts: Vec<&str> = base.split('-').collect();
    let mut path = root.to_path_buf();
    for i in 0..parts.len() {
        let component = parts[..=i].join("-");
        path = path.join(format!("{component}.slice"));
    }
    path
}

#[cfg(feature = "cgroups")]
fn make_cgroup_path(srvc_name: &str, slice: Option<&str>) -> Result<PathBuf, String> {
    let cgroup_root = crate::platform::cgroups::get_cgroup_root(&PathBuf::from("/sys/fs/cgroup"))
        .map_err(|e| format!("Couldnt get cgroup root: {}", e))?;
    let base = if let Some(slice_name) = slice {
        slice_cgroup_path(&cgroup_root, slice_name)
    } else {
        // Default to system.slice like real systemd
        slice_cgroup_path(&cgroup_root, "system.slice")
    };
    let service_cgroup = base.join(srvc_name);
    trace!(
        "Service {} will be moved into cgroup: {:?}",
        srvc_name, service_cgroup
    );
    Ok(service_cgroup)
}

#[cfg(not(feature = "cgroups"))]
fn make_cgroup_path(_srvc_name: &str, _slice: Option<&str>) -> Result<PathBuf, String> {
    // doesnt matter, wont be used anyways
    Ok(PathBuf::from("/ree"))
}

pub fn unit_from_parsed_service(conf: ParsedServiceConfig) -> Result<Unit, String> {
    // TODO make the cgroup path dynamic so multiple rust-systemd instances can exist
    let platform_specific = PlatformSpecificServiceFields {
        #[cfg(target_os = "linux")]
        cgroup_path: make_cgroup_path(&conf.common.name, conf.srvc.slice.as_deref())?,
    };

    let fragment_path = conf.common.fragment_path.clone();

    let mut sockets: Vec<UnitId> = Vec::new();
    for sock in conf.srvc.sockets {
        sockets.push(sock.as_str().try_into()?);
    }

    let mut common = make_common_from_parsed(conf.common.unit, conf.common.install, fragment_path)?;
    common.unit.refs_by_name.extend(sockets.iter().cloned());

    Ok(Unit {
        id: UnitId {
            kind: UnitIdKind::Service,
            name: conf.common.name,
        },
        common,
        specific: Specific::Service(ServiceSpecific {
            conf: ServiceConfig {
                exec_config: conf.srvc.exec_section.try_into()?,
                sockets,
                accept: conf.srvc.accept,
                dbus_name: conf.srvc.dbus_name,
                pid_file: conf.srvc.pid_file,
                restart: conf.srvc.restart,
                restart_sec: conf.srvc.restart_sec,
                kill_mode: conf.srvc.kill_mode,
                send_sighup: conf.srvc.send_sighup,
                delegate: conf.srvc.delegate,
                tasks_max: conf.srvc.tasks_max,
                limit_nofile: conf.srvc.limit_nofile,
                notifyaccess: conf.srvc.notifyaccess,
                exec: conf.srvc.exec,
                reload: conf.srvc.reload,
                startpre: conf.srvc.startpre,
                startpost: conf.srvc.startpost,
                stop: conf.srvc.stop,
                stoppost: conf.srvc.stoppost,
                srcv_type: conf.srvc.srcv_type,
                starttimeout: conf.srvc.starttimeout,
                stoptimeout: conf.srvc.stoptimeout,
                generaltimeout: conf.srvc.generaltimeout,
                platform_specific,
                slice: conf.srvc.slice,
                remain_after_exit: conf.srvc.remain_after_exit,
                success_exit_status: conf.srvc.success_exit_status,
                restart_force_exit_status: conf.srvc.restart_force_exit_status,
                memory_pressure_watch: conf.srvc.memory_pressure_watch,
                reload_signal: conf.srvc.reload_signal,
                kill_signal: conf.srvc.kill_signal,
                delegate_subgroup: conf.srvc.delegate_subgroup,
                keyring_mode: conf.srvc.keyring_mode,
                device_allow: conf.srvc.device_allow,
                device_policy: conf.srvc.device_policy,
                watchdog_sec: conf.srvc.watchdog_sec,
                ip_address_allow: conf.srvc.ip_address_allow,
                ip_address_deny: conf.srvc.ip_address_deny,
                file_descriptor_store_max: conf.srvc.file_descriptor_store_max,
                file_descriptor_store_preserve: conf.srvc.file_descriptor_store_preserve,
                memory_min: conf.srvc.memory_min,
                memory_low: conf.srvc.memory_low,
                memory_high: conf.srvc.memory_high,
                memory_max: conf.srvc.memory_max,
                memory_swap_max: conf.srvc.memory_swap_max,
                cpu_weight: conf.srvc.cpu_weight,
                startup_cpu_weight: conf.srvc.startup_cpu_weight,
                cpu_quota: conf.srvc.cpu_quota,
                io_weight: conf.srvc.io_weight,
                startup_io_weight: conf.srvc.startup_io_weight,
                io_device_weight: conf.srvc.io_device_weight,
                io_read_bandwidth_max: conf.srvc.io_read_bandwidth_max,
                io_write_bandwidth_max: conf.srvc.io_write_bandwidth_max,
                io_read_iops_max: conf.srvc.io_read_iops_max,
                io_write_iops_max: conf.srvc.io_write_iops_max,
                cpu_accounting: conf.srvc.cpu_accounting,
                memory_accounting: conf.srvc.memory_accounting,
                io_accounting: conf.srvc.io_accounting,
                tasks_accounting: conf.srvc.tasks_accounting,
                runtime_max_sec: conf.srvc.runtime_max_sec,
                coredump_receive: conf.srvc.coredump_receive,
                send_sigkill: conf.srvc.send_sigkill,
                restart_kill_signal: conf.srvc.restart_kill_signal,
                final_kill_signal: conf.srvc.final_kill_signal,
                watchdog_signal: conf.srvc.watchdog_signal,
                exit_type: conf.srvc.exit_type,
                oom_policy: conf.srvc.oom_policy,
                timeout_abort_sec: conf.srvc.timeout_abort_sec,
                timeout_clean_sec: conf.srvc.timeout_clean_sec,
                restart_prevent_exit_status: conf.srvc.restart_prevent_exit_status,
                restart_mode: conf.srvc.restart_mode,
                restart_steps: conf.srvc.restart_steps,
                restart_max_delay_sec: conf.srvc.restart_max_delay_sec,
                exec_condition: conf.srvc.exec_condition,
                // New service directives
                guess_main_pid: conf.srvc.guess_main_pid,
                timeout_start_failure_mode: conf.srvc.timeout_start_failure_mode,
                timeout_stop_failure_mode: conf.srvc.timeout_stop_failure_mode,
                runtime_randomized_extra_sec: conf.srvc.runtime_randomized_extra_sec,
                root_directory_start_only: conf.srvc.root_directory_start_only,
                non_blocking: conf.srvc.non_blocking,
                usb_function_descriptors: conf.srvc.usb_function_descriptors,
                usb_function_strings: conf.srvc.usb_function_strings,
                open_file: conf.srvc.open_file,
                // New resource-control directives
                cpu_quota_period_sec: conf.srvc.cpu_quota_period_sec,
                allowed_cpus: conf.srvc.allowed_cpus,
                startup_allowed_cpus: conf.srvc.startup_allowed_cpus,
                allowed_memory_nodes: conf.srvc.allowed_memory_nodes,
                startup_allowed_memory_nodes: conf.srvc.startup_allowed_memory_nodes,
                default_memory_min: conf.srvc.default_memory_min,
                default_memory_low: conf.srvc.default_memory_low,
                memory_zswap_max: conf.srvc.memory_zswap_max,
                io_device_latency_target_sec: conf.srvc.io_device_latency_target_sec,
                disable_controllers: conf.srvc.disable_controllers,
                memory_pressure_threshold_sec: conf.srvc.memory_pressure_threshold_sec,
                ip_ingress_filter_path: conf.srvc.ip_ingress_filter_path,
                ip_egress_filter_path: conf.srvc.ip_egress_filter_path,
                bpf_program: conf.srvc.bpf_program,
                socket_bind_allow: conf.srvc.socket_bind_allow,
                socket_bind_deny: conf.srvc.socket_bind_deny,
                restrict_network_interfaces: conf.srvc.restrict_network_interfaces,
                nft_set: conf.srvc.nft_set,
            },
            state: RwLock::new(ServiceState {
                common: CommonState::default(),
                srvc: Service {
                    pid: None,
                    main_pid: None,
                    status_msgs: Vec::new(),
                    process_group: None,
                    signaled_ready: false,
                    reloading: false,
                    stopping: false,
                    watchdog_last_ping: None,
                    notify_errno: None,
                    notify_bus_error: None,
                    notify_exit_status: None,
                    notify_monotonic_usec: None,
                    invocation_id: None,
                    watchdog_usec_override: None,
                    stored_fds: Vec::new(),
                    notify_access_override: None,
                    accepted_fd: None,
                    accepted_peer_uid: None,
                    notifications: None,
                    notifications_path: None,
                    stdout: None,
                    stderr: None,
                    journal_stream: None,
                    notifications_buffer: String::new(),
                    stdout_buffer: Vec::new(),
                    stderr_buffer: Vec::new(),
                    watchdog_timeout_fired: false,
                    runtime_max_timeout_fired: false,
                    runtime_started_at: None,
                    main_exit_status: None,
                    main_exit_pid: None,
                    trigger_path: None,
                    trigger_unit: None,
                    trigger_timer_realtime_usec: None,
                    trigger_timer_monotonic_usec: None,
                    monitor_env: None,
                    exec_main_start_timestamp: None,
                    exec_main_handoff_timestamp: None,
                    exec_main_exit_timestamp: None,
                    extend_timeout_usec: None,
                    extend_timeout_timestamp: None,
                    join_namespace_pid: None,
                    manual_stop: false,
                },
            }),
        }),
    })
}

pub fn unit_from_parsed_socket(conf: ParsedSocketConfig) -> Result<Unit, String> {
    let fragment_path = conf.common.fragment_path.clone();

    let mut services: Vec<UnitId> = Vec::new();
    for srvc in conf.sock.services {
        services.push(srvc.as_str().try_into()?);
    }

    let mut common = make_common_from_parsed(conf.common.unit, conf.common.install, fragment_path)?;
    common.unit.refs_by_name.extend(services.iter().cloned());

    Ok(Unit {
        id: UnitId {
            kind: UnitIdKind::Socket,
            name: conf.common.name,
        },
        common,
        specific: Specific::Socket(SocketSpecific {
            conf: SocketConfig {
                exec_config: conf.sock.exec_section.try_into()?,
                filedesc_name: conf
                    .sock
                    .filedesc_name
                    .unwrap_or_else(|| "unknown".to_owned()),
                services,
                sockets: conf.sock.sockets.into_iter().map(Into::into).collect(),
                accept: conf.sock.accept,
                max_connections: conf.sock.max_connections,
                max_connections_per_source: conf.sock.max_connections_per_source,
                socket_mode: conf.sock.socket_mode,
                directory_mode: conf.sock.directory_mode,
                pass_credentials: conf.sock.pass_credentials,
                pass_security: conf.sock.pass_security,
                accept_file_descriptors: conf.sock.accept_file_descriptors,
                remove_on_stop: conf.sock.remove_on_stop,
                receive_buffer: conf.sock.receive_buffer,
                send_buffer: conf.sock.send_buffer,
                symlinks: conf.sock.symlinks,
                timestamping: conf.sock.timestamping,
                defer_trigger: conf.sock.defer_trigger,
                writable: conf.sock.writable,
                backlog: conf.sock.backlog,
                bind_ipv6_only: conf.sock.bind_ipv6_only,
                bind_to_device: conf.sock.bind_to_device,
                socket_user: conf.sock.socket_user,
                socket_group: conf.sock.socket_group,
                free_bind: conf.sock.free_bind,
                transparent: conf.sock.transparent,
                broadcast: conf.sock.broadcast,
                reuse_port: conf.sock.reuse_port,
                keep_alive: conf.sock.keep_alive,
                keep_alive_time_sec: conf.sock.keep_alive_time_sec,
                keep_alive_interval_sec: conf.sock.keep_alive_interval_sec,
                keep_alive_probes: conf.sock.keep_alive_probes,
                no_delay: conf.sock.no_delay,
                priority: conf.sock.priority,
                mark: conf.sock.mark,
                ip_tos: conf.sock.ip_tos,
                ip_ttl: conf.sock.ip_ttl,
                pipe_size: conf.sock.pipe_size,
                flush_pending: conf.sock.flush_pending,
                trigger_limit_interval_sec: conf.sock.trigger_limit_interval_sec,
                trigger_limit_burst: conf.sock.trigger_limit_burst,
                poll_limit_interval_sec: conf.sock.poll_limit_interval_sec,
                poll_limit_burst: conf.sock.poll_limit_burst,
                socket_protocol: conf.sock.socket_protocol,
                selinux_context_from_net: conf.sock.selinux_context_from_net,
                smack_label: conf.sock.smack_label,
                smack_label_ipin: conf.sock.smack_label_ipin,
                smack_label_ipout: conf.sock.smack_label_ipout,
                pass_packet_info: conf.sock.pass_packet_info,
                tcp_congestion: conf.sock.tcp_congestion,
                exec_start_pre: conf.sock.exec_start_pre,
                exec_start_post: conf.sock.exec_start_post,
                exec_stop_pre: conf.sock.exec_stop_pre,
                exec_stop_post: conf.sock.exec_stop_post,
                timeout_sec: conf.sock.timeout_sec,
                pass_file_descriptors_to_exec: conf.sock.pass_file_descriptors_to_exec,
            },
            state: RwLock::new(SocketState {
                common: CommonState::default(),
                sock: Socket {
                    activated: false,
                    accept_counter: 0,
                    active_accept_connections: 0,
                    trigger_timestamps: Vec::new(),
                    poll_timestamps: Vec::new(),
                    poll_limit_paused_until: None,
                },
                result: SocketResult::Success,
            }),
        }),
    })
}
pub fn unit_from_parsed_target(conf: ParsedTargetConfig) -> Result<Unit, String> {
    let fragment_path = conf.common.fragment_path.clone();
    Ok(Unit {
        id: UnitId {
            kind: UnitIdKind::Target,
            name: conf.common.name,
        },
        common: make_common_from_parsed(conf.common.unit, conf.common.install, fragment_path)?,
        specific: Specific::Target(TargetSpecific {
            state: RwLock::new(TargetState {
                common: CommonState::default(),
            }),
        }),
    })
}

pub fn unit_from_parsed_slice(conf: ParsedSliceConfig) -> Result<Unit, String> {
    let fragment_path = conf.common.fragment_path.clone();
    Ok(Unit {
        id: UnitId {
            kind: UnitIdKind::Slice,
            name: conf.common.name,
        },
        common: make_common_from_parsed(conf.common.unit, conf.common.install, fragment_path)?,
        specific: Specific::Slice(SliceSpecific {
            conf: SliceConfig::from(conf.slice),
            state: RwLock::new(SliceState {
                common: CommonState::default(),
            }),
        }),
    })
}

pub fn unit_from_parsed_mount(conf: ParsedMountConfig) -> Result<Unit, String> {
    let fragment_path = conf.common.fragment_path.clone();
    Ok(Unit {
        id: UnitId {
            kind: UnitIdKind::Mount,
            name: conf.common.name,
        },
        common: make_common_from_parsed(conf.common.unit, conf.common.install, fragment_path)?,
        specific: Specific::Mount(MountSpecific {
            conf: MountConfig::from(conf.mount),
            state: RwLock::new(MountState {
                common: CommonState::default(),
            }),
        }),
    })
}

pub fn unit_from_parsed_swap(conf: ParsedSwapConfig) -> Result<Unit, String> {
    let fragment_path = conf.common.fragment_path.clone();
    Ok(Unit {
        id: UnitId {
            kind: UnitIdKind::Swap,
            name: conf.common.name,
        },
        common: make_common_from_parsed(conf.common.unit, conf.common.install, fragment_path)?,
        specific: Specific::Swap(SwapSpecific {
            conf: SwapConfig::from(conf.swap),
            state: RwLock::new(SwapState {
                common: CommonState::default(),
            }),
        }),
    })
}

pub fn unit_from_parsed_timer(conf: ParsedTimerConfig) -> Result<Unit, String> {
    let fragment_path = conf.common.fragment_path.clone();
    let timer_name = &conf.common.name;

    // Determine the unit to activate: explicit Unit= or same-name .service
    let target_unit = conf.timer.unit.clone().unwrap_or_else(|| {
        timer_name
            .strip_suffix(".timer")
            .map(|base| format!("{base}.service"))
            .unwrap_or_else(|| format!("{timer_name}.service"))
    });

    let timer_conf = TimerConfig {
        on_active_sec: conf
            .timer
            .on_active_sec
            .iter()
            .filter_map(|s| parse_timespan(s))
            .collect(),
        on_boot_sec: conf
            .timer
            .on_boot_sec
            .iter()
            .filter_map(|s| parse_timespan(s))
            .collect(),
        on_startup_sec: conf
            .timer
            .on_startup_sec
            .iter()
            .filter_map(|s| parse_timespan(s))
            .collect(),
        on_unit_active_sec: conf
            .timer
            .on_unit_active_sec
            .iter()
            .filter_map(|s| parse_timespan(s))
            .collect(),
        on_unit_inactive_sec: conf
            .timer
            .on_unit_inactive_sec
            .iter()
            .filter_map(|s| parse_timespan(s))
            .collect(),
        on_calendar: conf.timer.on_calendar.clone(),
        accuracy_sec: conf
            .timer
            .accuracy_sec
            .as_deref()
            .and_then(parse_timespan)
            .unwrap_or(std::time::Duration::from_secs(60)),
        randomized_delay_sec: conf
            .timer
            .randomized_delay_sec
            .as_deref()
            .and_then(parse_timespan)
            .unwrap_or(std::time::Duration::ZERO),
        fixed_random_delay: conf.timer.fixed_random_delay,
        persistent: conf.timer.persistent,
        wake_system: conf.timer.wake_system,
        remain_after_elapse: conf.timer.remain_after_elapse,
        on_clock_change: conf.timer.on_clock_change,
        on_timezone_change: conf.timer.on_timezone_change,
        unit: target_unit,
    };

    Ok(Unit {
        id: UnitId {
            kind: UnitIdKind::Timer,
            name: conf.common.name,
        },
        common: make_common_from_parsed(conf.common.unit, conf.common.install, fragment_path)?,
        specific: Specific::Timer(TimerSpecific {
            conf: timer_conf,
            state: RwLock::new(TimerState {
                common: CommonState::default(),
                last_trigger_usec: None,
            }),
        }),
    })
}

pub fn unit_from_parsed_path(conf: ParsedPathConfig) -> Result<Unit, String> {
    let fragment_path = conf.common.fragment_path.clone();
    let path_name = &conf.common.name;

    // Determine the unit to activate: explicit Unit= or same-name .service
    let target_unit = conf.path.unit.clone().unwrap_or_else(|| {
        path_name
            .strip_suffix(".path")
            .map(|base| format!("{base}.service"))
            .unwrap_or_else(|| format!("{path_name}.service"))
    });

    // Convert parsed path conditions to PathCondition enums
    let mut conditions = Vec::new();
    for (kind, value) in &conf.path.path_exists {
        match kind.as_str() {
            "PathExists" => conditions.push(PathCondition::PathExists(value.clone())),
            "PathExistsGlob" => conditions.push(PathCondition::PathExistsGlob(value.clone())),
            "PathChanged" => conditions.push(PathCondition::PathChanged(value.clone())),
            "PathModified" => conditions.push(PathCondition::PathModified(value.clone())),
            "DirectoryNotEmpty" => {
                conditions.push(PathCondition::DirectoryNotEmpty(value.clone()));
            }
            _ => {
                trace!("Unknown path condition kind: {kind}={value}");
            }
        }
    }

    let path_conf = PathConfig {
        conditions,
        make_directory: conf.path.make_directory,
        directory_mode: conf.path.directory_mode,
        trigger_limit_interval_sec: conf
            .path
            .trigger_limit_interval_sec
            .as_deref()
            .and_then(parse_timespan)
            .unwrap_or(std::time::Duration::from_secs(2)),
        trigger_limit_burst: conf.path.trigger_limit_burst.unwrap_or(200),
        unit: target_unit,
    };

    Ok(Unit {
        id: UnitId {
            kind: UnitIdKind::Path,
            name: conf.common.name,
        },
        common: make_common_from_parsed(conf.common.unit, conf.common.install, fragment_path)?,
        specific: Specific::Path(PathSpecific {
            conf: path_conf,
            state: RwLock::new(PathState {
                common: CommonState::default(),
                result: crate::units::PathResult::default(),
            }),
        }),
    })
}

/// Parse a systemd timespan string (e.g. "15min", "1h 30min", "2s", "1d 6h")
/// into a `Duration`. Supports the suffixes: us/usec, ms/msec, s/sec/seconds,
/// min/minutes, h/hr/hour/hours, d/day/days, w/week/weeks, month/months,
/// y/year/years. A bare number is treated as seconds. Returns `None` on parse error.
pub fn parse_timespan(input: &str) -> Option<std::time::Duration> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    // Try bare number first (seconds)
    if let Ok(secs) = input.parse::<f64>() {
        if secs >= 0.0 {
            return Some(std::time::Duration::from_secs_f64(secs));
        }
        return None;
    }

    let mut total_us: u64 = 0;
    let mut chars = input.chars().peekable();

    while chars.peek().is_some() {
        // Skip whitespace
        while chars.peek() == Some(&' ') {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }

        // Parse number
        let mut num_str = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() || c == '.' {
                num_str.push(c);
                chars.next();
            } else {
                break;
            }
        }
        if num_str.is_empty() {
            return None;
        }
        let num: f64 = num_str.parse().ok()?;

        // Skip whitespace between number and unit
        while chars.peek() == Some(&' ') {
            chars.next();
        }

        // Parse unit suffix
        let mut unit = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_alphabetic() {
                unit.push(c);
                chars.next();
            } else {
                break;
            }
        }

        let multiplier_us: u64 = match unit.as_str() {
            "us" | "usec" => 1,
            "ms" | "msec" => 1_000,
            "" | "s" | "sec" | "second" | "seconds" | "secs" => 1_000_000,
            "min" | "minute" | "minutes" | "m" => 60 * 1_000_000,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600 * 1_000_000,
            "d" | "day" | "days" => 86400 * 1_000_000,
            "w" | "week" | "weeks" => 7 * 86400 * 1_000_000,
            "month" | "months" => 30 * 86400 * 1_000_000, // approximate
            "y" | "year" | "years" => 365 * 86400 * 1_000_000, // approximate
            _ => return None,
        };

        total_us += (num * multiplier_us as f64) as u64;
    }

    Some(std::time::Duration::from_micros(total_us))
}

impl From<ParsedSingleSocketConfig> for SingleSocketConfig {
    fn from(parsed: ParsedSingleSocketConfig) -> Self {
        Self {
            kind: parsed.kind,
            specialized: parsed.specialized,
        }
    }
}

impl std::convert::TryFrom<ParsedExecSection> for ExecConfig {
    type Error = String;
    fn try_from(parsed: ParsedExecSection) -> Result<Self, String> {
        // Store raw user/group strings and defer resolution to exec time.
        // This matches systemd's behavior: users/groups that don't exist yet
        // at unit-load time (e.g. created by systemd-sysusers) are resolved
        // just before the service process is spawned.
        Ok(Self {
            user: parsed.user,
            group: parsed.group,
            supplementary_groups: parsed.supplementary_groups,
            stdin_option: parsed.stdin_option,
            stderr_path: parsed.stderr_path,
            stdout_path: parsed.stdout_path,
            environment: parsed.environment,
            environment_files: parsed.environment_files,
            working_directory: parsed.working_directory,
            state_directory: parsed.state_directory,
            logs_directory: parsed.logs_directory,
            logs_directory_mode: parsed.logs_directory_mode,
            runtime_directory: parsed.runtime_directory,
            runtime_directory_preserve: parsed.runtime_directory_preserve,
            tty_path: parsed.tty_path,
            tty_reset: parsed.tty_reset,
            tty_vhangup: parsed.tty_vhangup,
            tty_vt_disallocate: parsed.tty_vt_disallocate,
            ignore_sigpipe: parsed.ignore_sigpipe,
            utmp_identifier: parsed.utmp_identifier,
            utmp_mode: parsed.utmp_mode,
            import_credentials: parsed.import_credentials,
            load_credentials: parsed.load_credentials,
            load_credentials_encrypted: parsed.load_credentials_encrypted,
            set_credentials: parsed.set_credentials,
            set_credentials_encrypted: parsed.set_credentials_encrypted,
            pass_environment: parsed.pass_environment,
            unset_environment: parsed.unset_environment,
            oom_score_adjust: parsed.oom_score_adjust,
            log_extra_fields: parsed.log_extra_fields,
            dynamic_user: parsed.dynamic_user,
            system_call_filter: parsed.system_call_filter,
            system_call_log: parsed.system_call_log,
            protect_system: parsed.protect_system,
            restrict_namespaces: parsed.restrict_namespaces,
            restrict_realtime: parsed.restrict_realtime,
            restrict_address_families: parsed.restrict_address_families,
            restrict_file_systems: parsed.restrict_file_systems,
            system_call_error_number: parsed.system_call_error_number,
            no_new_privileges: parsed.no_new_privileges,
            protect_control_groups: parsed.protect_control_groups,
            protect_control_groups_ex: parsed.protect_control_groups_ex,
            protect_kernel_modules: parsed.protect_kernel_modules,
            restrict_suid_sgid: parsed.restrict_suid_sgid,
            protect_kernel_logs: parsed.protect_kernel_logs,
            protect_kernel_tunables: parsed.protect_kernel_tunables,
            protect_clock: parsed.protect_clock,
            capability_bounding_set: parsed.capability_bounding_set,
            ambient_capabilities: parsed.ambient_capabilities,
            protect_home: parsed.protect_home,
            protect_hostname: parsed.protect_hostname,
            protect_hostname_mode: None, // Extended syntax only via transient units
            protect_hostname_name: None, // Extended syntax only via transient units
            system_call_architectures: parsed.system_call_architectures,
            read_write_paths: parsed.read_write_paths,
            memory_deny_write_execute: parsed.memory_deny_write_execute,
            lock_personality: parsed.lock_personality,
            protect_proc: parsed.protect_proc,
            private_tmp: parsed.private_tmp,
            private_devices: parsed.private_devices,
            private_network: parsed.private_network,
            private_users: parsed.private_users,
            private_mounts: parsed.private_mounts,
            mount_flags: None, // Not parsed from unit files yet
            io_scheduling_class: parsed.io_scheduling_class,
            io_scheduling_priority: parsed.io_scheduling_priority,
            umask: parsed.umask,
            proc_subset: parsed.proc_subset,
            nice: parsed.nice,
            remove_ipc: parsed.remove_ipc,
            pam_name: parsed.pam_name,

            // Resource limits
            limit_cpu: parsed.limit_cpu,
            limit_core: parsed.limit_core,
            limit_fsize: parsed.limit_fsize,
            limit_data: parsed.limit_data,
            limit_stack: parsed.limit_stack,
            limit_rss: parsed.limit_rss,
            limit_nproc: parsed.limit_nproc,
            limit_memlock: parsed.limit_memlock,
            limit_as: parsed.limit_as,
            limit_locks: parsed.limit_locks,
            limit_sigpending: parsed.limit_sigpending,
            limit_msgqueue: parsed.limit_msgqueue,
            limit_nice: parsed.limit_nice,
            limit_rtprio: parsed.limit_rtprio,
            limit_rttime: parsed.limit_rttime,

            // Directory management
            cache_directory: parsed.cache_directory,
            cache_directory_mode: parsed.cache_directory_mode,
            configuration_directory: parsed.configuration_directory,
            configuration_directory_mode: parsed.configuration_directory_mode,
            state_directory_mode: parsed.state_directory_mode,
            runtime_directory_mode: parsed.runtime_directory_mode,

            // Path-based mount namespace directives
            read_only_paths: parsed.read_only_paths,
            inaccessible_paths: parsed.inaccessible_paths,
            bind_paths: parsed.bind_paths,
            bind_read_only_paths: parsed.bind_read_only_paths,
            temporary_file_system: parsed.temporary_file_system,

            // Logging directives
            syslog_identifier: parsed.syslog_identifier,

            // Additional logging directives
            syslog_facility: parsed.syslog_facility,
            syslog_level: parsed.syslog_level,
            syslog_level_prefix: parsed.syslog_level_prefix,
            log_level_max: parsed.log_level_max,
            log_rate_limit_interval_sec: parsed.log_rate_limit_interval_sec,
            log_rate_limit_burst: parsed.log_rate_limit_burst,
            log_filter_patterns: parsed.log_filter_patterns,
            log_namespace: parsed.log_namespace,

            // CPU scheduling directives
            cpu_scheduling_policy: parsed.cpu_scheduling_policy,
            cpu_scheduling_priority: parsed.cpu_scheduling_priority,
            cpu_scheduling_reset_on_fork: parsed.cpu_scheduling_reset_on_fork,
            cpu_affinity: parsed.cpu_affinity,
            numa_policy: parsed.numa_policy,
            numa_mask: parsed.numa_mask,

            // Root filesystem / image directives
            root_directory: parsed.root_directory,
            root_image: parsed.root_image,
            root_image_options: parsed.root_image_options,
            root_hash: parsed.root_hash,
            root_hash_signature: parsed.root_hash_signature,
            root_verity: parsed.root_verity,
            root_ephemeral: parsed.root_ephemeral,
            mount_api_vfs: parsed.mount_api_vfs,
            extension_directories: parsed.extension_directories,
            extension_images: parsed.extension_images,
            mount_images: parsed.mount_images,
            bind_log_sockets: parsed.bind_log_sockets,

            // Additional namespace directives
            private_ipc: parsed.private_ipc,
            private_pids: parsed.private_pids,
            ipc_namespace_path: parsed.ipc_namespace_path,
            network_namespace_path: parsed.network_namespace_path,

            // Security directives
            secure_bits: parsed.secure_bits,
            personality: parsed.personality,
            selinux_context: parsed.selinux_context,
            apparmor_profile: parsed.apparmor_profile,
            smack_process_label: parsed.smack_process_label,
            keyring_mode_exec: parsed.keyring_mode,
            no_exec_paths: parsed.no_exec_paths,
            exec_paths: parsed.exec_paths,
            coredump_filter: parsed.coredump_filter,

            // Misc directives
            timer_slack_nsec: parsed.timer_slack_nsec,
            standard_input_text: parsed.standard_input_text,
            standard_input_data: parsed.standard_input_data,
            set_login_environment: parsed.set_login_environment,
        })
    }
}

/// Convert a list of unit name strings into UnitIds, skipping any with
/// unsupported suffixes (e.g. .scope, .automount).
/// This matches systemd's behavior of silently ignoring unit types it
/// doesn't manage in dependency lists, rather than rejecting the
/// entire unit file.
fn collect_supported_unit_ids(names: Vec<String>) -> Vec<UnitId> {
    let mut ids = Vec::new();
    for name in names {
        match <&str as TryInto<UnitId>>::try_into(name.as_str()) {
            Ok(id) => ids.push(id),
            Err(_) => {
                trace!(
                    "Skipping unsupported unit type in dependency list: {}",
                    name
                );
            }
        }
    }
    ids
}

fn make_common_from_parsed(
    unit: ParsedUnitSection,
    install: ParsedInstallSection,
    fragment_path: Option<std::path::PathBuf>,
) -> Result<Common, String> {
    let mut wants = collect_supported_unit_ids(unit.wants);
    // Also= in [Install] is treated as a soft (wants) dependency
    wants.extend(collect_supported_unit_ids(install.also));
    let requires = collect_supported_unit_ids(unit.requires);
    let binds_to = collect_supported_unit_ids(unit.binds_to);
    let upholds = collect_supported_unit_ids(unit.upholds);
    let propagates_stop_to = collect_supported_unit_ids(unit.propagates_stop_to);
    let part_of = collect_supported_unit_ids(unit.part_of);
    let conflicts = collect_supported_unit_ids(unit.conflicts);
    let wanted_by = collect_supported_unit_ids(install.wanted_by);
    let required_by = collect_supported_unit_ids(install.required_by);
    let after = collect_supported_unit_ids(unit.after);
    let before = collect_supported_unit_ids(unit.before);

    let mut refs_by_name = Vec::new();
    refs_by_name.extend(wants.iter().cloned());
    refs_by_name.extend(wanted_by.iter().cloned());
    refs_by_name.extend(requires.iter().cloned());
    refs_by_name.extend(required_by.iter().cloned());
    refs_by_name.extend(part_of.iter().cloned());
    refs_by_name.extend(binds_to.iter().cloned());
    refs_by_name.extend(upholds.iter().cloned());
    refs_by_name.extend(propagates_stop_to.iter().cloned());
    refs_by_name.extend(conflicts.iter().cloned());
    refs_by_name.extend(before.iter().cloned());
    refs_by_name.extend(after.iter().cloned());

    Ok(Common {
        status: RwLock::new(UnitStatus::NeverStarted),
        unit: UnitConfig {
            description: unit.description,
            documentation: unit.documentation,
            fragment_path,
            refs_by_name,
            default_dependencies: unit.default_dependencies,
            ignore_on_isolate: unit.ignore_on_isolate,
            conditions: unit.conditions,
            assertions: unit.assertions,
            success_action: unit.success_action,
            failure_action: unit.failure_action,
            job_timeout_action: unit.job_timeout_action,
            job_timeout_sec: unit.job_timeout_sec,
            allow_isolate: unit.allow_isolate,
            refuse_manual_start: unit.refuse_manual_start,
            refuse_manual_stop: unit.refuse_manual_stop,
            on_success: unit.on_success,
            on_success_job_mode: unit.on_success_job_mode,
            on_failure: unit.on_failure,
            on_failure_job_mode: unit.on_failure_job_mode,
            start_limit_interval_sec: unit.start_limit_interval_sec,
            start_limit_burst: unit.start_limit_burst,
            start_limit_action: unit.start_limit_action,
            aliases: install.alias,
            default_instance: install.default_instance,
            loaded_at: std::time::SystemTime::now(),
            loaded_dropin_files: Vec::new(),
            joins_namespace_of: unit.joins_namespace_of.clone(),
        },
        dependencies: Dependencies {
            wants,
            wanted_by,
            requires,
            required_by,
            conflicts,
            conflicted_by: Vec::new(),
            before,
            after,
            part_of,
            part_of_by: Vec::new(),
            binds_to,
            bound_by: Vec::new(),
            upholds,
            upheld_by: Vec::new(),
            propagates_stop_to,
        },
        timestamps: RwLock::new(UnitTimestamps::default()),
        n_restarts: std::sync::atomic::AtomicU64::new(0),
        deactivation_in_progress: std::sync::atomic::AtomicBool::new(false),
        deactivation_irreversible: std::sync::atomic::AtomicBool::new(false),
        start_requested_during_deactivation: std::sync::atomic::AtomicBool::new(false),
        invocation_id: std::sync::Mutex::new(String::new()),
        main_pid: std::sync::atomic::AtomicI32::new(0),
        main_exit_pid: std::sync::atomic::AtomicI32::new(0),
        main_exit_status: std::sync::atomic::AtomicI32::new(-1),
    })
}

impl std::convert::TryInto<UnitId> for &str {
    type Error = String;
    fn try_into(self) -> Result<UnitId, String> {
        if self.ends_with(".target") {
            Ok(UnitId {
                name: self.to_owned(),
                kind: UnitIdKind::Target,
            })
        } else if self.ends_with(".service") {
            Ok(UnitId {
                name: self.to_owned(),
                kind: UnitIdKind::Service,
            })
        } else if self.ends_with(".socket") {
            Ok(UnitId {
                name: self.to_owned(),
                kind: UnitIdKind::Socket,
            })
        } else if self.ends_with(".slice") {
            Ok(UnitId {
                name: self.to_owned(),
                kind: UnitIdKind::Slice,
            })
        } else if self.ends_with(".mount") {
            Ok(UnitId {
                name: self.to_owned(),
                kind: UnitIdKind::Mount,
            })
        } else if self.ends_with(".device") {
            Ok(UnitId {
                name: self.to_owned(),
                kind: UnitIdKind::Device,
            })
        } else if self.ends_with(".timer") {
            Ok(UnitId {
                name: self.to_owned(),
                kind: UnitIdKind::Timer,
            })
        } else if self.ends_with(".path") {
            Ok(UnitId {
                name: self.to_owned(),
                kind: UnitIdKind::Path,
            })
        } else if self.ends_with(".swap") {
            Ok(UnitId {
                name: self.to_owned(),
                kind: UnitIdKind::Swap,
            })
        } else {
            Err(format!(
                "{self} is not a valid unit name. The suffix is not supported."
            ))
        }
    }
}

impl std::convert::TryFrom<ParsedServiceConfig> for Unit {
    type Error = String;
    fn try_from(conf: ParsedServiceConfig) -> Result<Self, String> {
        unit_from_parsed_service(conf)
    }
}
impl std::convert::TryFrom<ParsedSocketConfig> for Unit {
    type Error = String;
    fn try_from(conf: ParsedSocketConfig) -> Result<Self, String> {
        unit_from_parsed_socket(conf)
    }
}
impl std::convert::TryFrom<ParsedTargetConfig> for Unit {
    type Error = String;
    fn try_from(conf: ParsedTargetConfig) -> Result<Self, String> {
        unit_from_parsed_target(conf)
    }
}
impl std::convert::TryFrom<ParsedSliceConfig> for Unit {
    type Error = String;
    fn try_from(conf: ParsedSliceConfig) -> Result<Self, String> {
        unit_from_parsed_slice(conf)
    }
}
impl std::convert::TryFrom<ParsedMountConfig> for Unit {
    type Error = String;
    fn try_from(conf: ParsedMountConfig) -> Result<Self, String> {
        unit_from_parsed_mount(conf)
    }
}
impl std::convert::TryFrom<ParsedSwapConfig> for Unit {
    type Error = String;
    fn try_from(conf: ParsedSwapConfig) -> Result<Self, String> {
        unit_from_parsed_swap(conf)
    }
}
impl std::convert::TryFrom<ParsedTimerConfig> for Unit {
    type Error = String;
    fn try_from(conf: ParsedTimerConfig) -> Result<Self, String> {
        unit_from_parsed_timer(conf)
    }
}
impl std::convert::TryFrom<ParsedPathConfig> for Unit {
    type Error = String;
    fn try_from(conf: ParsedPathConfig) -> Result<Self, String> {
        unit_from_parsed_path(conf)
    }
}
impl std::convert::TryFrom<ParsedDeviceConfig> for Unit {
    type Error = String;
    fn try_from(conf: ParsedDeviceConfig) -> Result<Self, String> {
        unit_from_parsed_device(conf)
    }
}

pub fn unit_from_parsed_device(conf: ParsedDeviceConfig) -> Result<Unit, String> {
    let fragment_path = conf.common.fragment_path.clone();
    Ok(Unit {
        id: UnitId {
            kind: UnitIdKind::Device,
            name: conf.common.name,
        },
        common: make_common_from_parsed(conf.common.unit, conf.common.install, fragment_path)?,
        specific: Specific::Device(DeviceSpecific {
            conf: DeviceConfig {
                sysfs_path: conf.sysfs_path,
            },
            state: RwLock::new(DeviceState {
                common: CommonState::default(),
            }),
        }),
    })
}

/// Create a device unit dynamically at runtime, as would happen when udev
/// reports a new device tagged with `TAG+="systemd"`.
///
/// This is the primary entry point for integrating udev events with the
/// service manager. When a device appears, udev rules can set:
///
/// - `SYSTEMD_ALIAS=` — additional `.device` unit names for the device
/// - `SYSTEMD_WANTS=` — units the device should pull in via `Wants=`
/// - `SYSTEMD_READY=` — whether the device is considered "ready" (default: true)
/// - `TAG+="systemd"` — marks the device for tracking by the service manager
///
/// # Arguments
///
/// * `unit_name` — The escaped device unit name (e.g., `dev-sda1.device`,
///   `sys-subsystem-net-devices-eth0.device`).
/// * `sysfs_path` — The sysfs path of the device (e.g., `/sys/devices/pci0000:00/...`).
/// * `wants` — Unit names from `SYSTEMD_WANTS=` that this device should pull in.
/// * `description` — Human-readable description (e.g., from `DEVNAME` or sysfs).
#[allow(dead_code)]
pub fn create_device_unit(
    unit_name: &str,
    sysfs_path: Option<String>,
    wants: &[String],
    description: Option<String>,
) -> Result<Unit, String> {
    let mut unit_section = ParsedUnitSection::default();
    if let Some(desc) = description {
        unit_section.description = desc;
    }
    for want in wants {
        unit_section.wants.push(want.clone());
    }
    // Device units should not get default dependencies — they are managed
    // by udev and should not implicitly depend on sysinit.target etc.
    unit_section.default_dependencies = false;

    let conf = ParsedDeviceConfig {
        common: ParsedCommonConfig {
            name: unit_name.to_owned(),
            unit: unit_section,
            install: Default::default(),
            fragment_path: None,
        },
        sysfs_path,
    };
    unit_from_parsed_device(conf)
}

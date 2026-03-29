use log::trace;

use crate::units::{
    Commandline, CommandlinePrefix, Delegate, KeyringMode, KillMode, MemoryPressureWatch,
    NotifyKind, ParsedCommonConfig, ParsedFile, ParsedSection, ParsedServiceConfig,
    ParsedServiceSection, ParsingErrorReason, RLimitValue, ResourceLimit, ServiceRestart,
    ServiceType, SuccessExitStatus, TasksMax, Timeout, map_tuples_to_second, parse_cpu_quota,
    parse_cpu_weight, parse_install_section, parse_io_device_limit, parse_io_weight,
    parse_memory_limit, parse_unit_section, string_to_bool,
};
use std::collections::HashMap;
use std::path::PathBuf;

/// Parse a signal name string (with or without `SIG` prefix) into a
/// `nix::sys::signal::Signal`.  Accepts e.g. `SIGTERM`, `TERM`, `SIGHUP`,
/// `HUP`, etc.  Returns `None` for unrecognised names.
fn parse_signal_name(s: &str) -> Option<nix::sys::signal::Signal> {
    use nix::sys::signal::Signal;
    let upper = s.to_uppercase();
    let name = upper.strip_prefix("SIG").unwrap_or(&upper);
    match name {
        "HUP" => Some(Signal::SIGHUP),
        "INT" => Some(Signal::SIGINT),
        "QUIT" => Some(Signal::SIGQUIT),
        "ILL" => Some(Signal::SIGILL),
        "TRAP" => Some(Signal::SIGTRAP),
        "ABRT" | "IOT" => Some(Signal::SIGABRT),
        "BUS" => Some(Signal::SIGBUS),
        "FPE" => Some(Signal::SIGFPE),
        "KILL" => Some(Signal::SIGKILL),
        "USR1" => Some(Signal::SIGUSR1),
        "SEGV" => Some(Signal::SIGSEGV),
        "USR2" => Some(Signal::SIGUSR2),
        "PIPE" => Some(Signal::SIGPIPE),
        "ALRM" => Some(Signal::SIGALRM),
        "TERM" => Some(Signal::SIGTERM),
        "CHLD" => Some(Signal::SIGCHLD),
        "CONT" => Some(Signal::SIGCONT),
        "STOP" => Some(Signal::SIGSTOP),
        "TSTP" => Some(Signal::SIGTSTP),
        "TTIN" => Some(Signal::SIGTTIN),
        "TTOU" => Some(Signal::SIGTTOU),
        "URG" => Some(Signal::SIGURG),
        "XCPU" => Some(Signal::SIGXCPU),
        "XFSZ" => Some(Signal::SIGXFSZ),
        "VTALRM" => Some(Signal::SIGVTALRM),
        "PROF" => Some(Signal::SIGPROF),
        "WINCH" => Some(Signal::SIGWINCH),
        "IO" => Some(Signal::SIGIO),
        "SYS" => Some(Signal::SIGSYS),
        "PWR" => Some(Signal::SIGPWR),
        _ => None,
    }
}

/// Linux kernel SIGRTMIN (before glibc reservations).  Systemd uses this
/// value for `RTMIN+N` parsing.
const SIGRTMIN_RAW: i32 = 34;
/// Linux kernel SIGRTMAX.
const SIGRTMAX_RAW: i32 = 64;

/// Parse a signal specification into a raw signal number (`i32`).
///
/// Accepts everything `parse_signal_name` accepts, plus:
/// - Plain integers (`"15"`, `"59"`)
/// - `RTMIN`, `RTMIN+N`, `SIGRTMIN+N`
/// - `RTMAX`, `RTMAX-N`, `SIGRTMAX-N`
///
/// Returns `None` for unrecognised values.
pub fn parse_signal_to_raw(s: &str) -> Option<i32> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 1. Try standard named signals
    if let Some(sig) = parse_signal_name(trimmed) {
        return Some(sig as i32);
    }

    // 2. Try plain numeric
    if let Ok(num) = trimmed.parse::<i32>() {
        if num > 0 && num <= SIGRTMAX_RAW {
            return Some(num);
        }
        return None;
    }

    // 3. Try RTMIN / RTMIN+N / RTMAX / RTMAX-N  (with optional SIG prefix)
    let upper = trimmed.to_uppercase();
    let name = upper.strip_prefix("SIG").unwrap_or(&upper);

    if name == "RTMIN" {
        return Some(SIGRTMIN_RAW);
    }
    if name == "RTMAX" {
        return Some(SIGRTMAX_RAW);
    }
    if let Some(offset_str) = name.strip_prefix("RTMIN+") {
        if let Ok(offset) = offset_str.trim().parse::<i32>() {
            let sig = SIGRTMIN_RAW + offset;
            if (SIGRTMIN_RAW..=SIGRTMAX_RAW).contains(&sig) {
                return Some(sig);
            }
        }
        return None;
    }
    if let Some(offset_str) = name.strip_prefix("RTMAX-") {
        if let Ok(offset) = offset_str.trim().parse::<i32>() {
            let sig = SIGRTMAX_RAW - offset;
            if (SIGRTMIN_RAW..=SIGRTMAX_RAW).contains(&sig) {
                return Some(sig);
            }
        }
        return None;
    }

    None
}

/// Parse a named exit status (BSD sysexits.h, C library, LSB, and
/// systemd-specific codes).  Returns the numeric exit code if recognised.
///
/// Accepts both the canonical symbolic name (e.g. `DATAERR`) and the
/// `EX_`-prefixed form (e.g. `EX_DATAERR`), as well as `EXIT_`-prefixed
/// names (e.g. `EXIT_SUCCESS`, `EXIT_FAILURE`).  Matching is
/// case-insensitive.
fn parse_exit_status_name(s: &str) -> Option<i32> {
    // Strip optional EX_ or EXIT_ prefix for lookup
    let upper = s.to_uppercase();
    let name = if let Some(rest) = upper.strip_prefix("EX_") {
        rest
    } else if let Some(rest) = upper.strip_prefix("EXIT_") {
        rest
    } else {
        &upper
    };

    match name {
        // C library (Table 8)
        "SUCCESS" => Some(0),
        "FAILURE" => Some(1),

        // LSB (Table 9)
        "INVALIDARGUMENT" => Some(2),
        "NOTIMPLEMENTED" => Some(3),
        "NOPERMISSION" => Some(4),
        "NOTINSTALLED" => Some(5),
        "NOTCONFIGURED" => Some(6),
        "NOTRUNNING" => Some(7),

        // BSD sysexits.h (Table 11)
        "USAGE" => Some(64),
        "DATAERR" => Some(65),
        "NOINPUT" => Some(66),
        "NOUSER" => Some(67),
        "NOHOST" => Some(68),
        "UNAVAILABLE" => Some(69),
        "SOFTWARE" => Some(70),
        "OSERR" => Some(71),
        "OSFILE" => Some(72),
        "CANTCREAT" => Some(73),
        "IOERR" => Some(74),
        "TEMPFAIL" => Some(75),
        "PROTOCOL" => Some(76),
        "NOPERM" => Some(77),
        "CONFIG" => Some(78),

        // systemd-specific (Table 10)
        "CHDIR" => Some(200),
        "NICE" => Some(201),
        "FDS" => Some(202),
        "EXEC" => Some(203),
        "MEMORY" => Some(204),
        "LIMITS" => Some(205),
        "OOM_ADJUST" => Some(206),
        "SIGNAL_MASK" => Some(207),
        "STDIN" => Some(208),
        "STDOUT" => Some(209),
        "CHROOT" => Some(210),
        "IOPRIO" => Some(211),
        "TIMERSLACK" => Some(212),
        "SECUREBITS" => Some(213),
        "SETSCHEDULER" => Some(214),
        "CPUAFFINITY" => Some(215),
        "GROUP" => Some(216),
        "USER" => Some(217),
        "CAPABILITIES" => Some(218),
        "CGROUP" => Some(219),
        "SETSID" => Some(220),
        "CONFIRM" => Some(221),
        "STDERR" => Some(222),
        "PAM" => Some(224),
        "NETWORK" => Some(225),
        "NAMESPACE" => Some(226),
        "NO_NEW_PRIVILEGES" => Some(227),
        "SECCOMP" => Some(228),
        "SELINUX_CONTEXT" => Some(229),
        "PERSONALITY" => Some(230),
        "APPARMOR_PROFILE" => Some(231),
        "ADDRESS_FAMILIES" => Some(232),
        "RUNTIME_DIRECTORY" => Some(233),
        "CHOWN" => Some(235),
        "SMACK_PROCESS_LABEL" => Some(236),
        "KEYRING" => Some(237),
        "STATE_DIRECTORY" => Some(238),
        "CACHE_DIRECTORY" => Some(239),
        "LOGS_DIRECTORY" => Some(240),
        "CONFIGURATION_DIRECTORY" => Some(241),
        "NUMA_POLICY" => Some(242),
        "CREDENTIALS" => Some(243),
        "BPF" => Some(245),

        _ => None,
    }
}

/// Parse a `SuccessExitStatus=` value.  The value is a space-separated list
/// of tokens.  Numeric tokens are treated as exit codes; named tokens are
/// tried first as exit status names (BSD sysexits, LSB, systemd-specific),
/// then as signal names.
fn parse_success_exit_status(raw: &str) -> SuccessExitStatus {
    let mut exit_codes = Vec::new();
    let mut signals = Vec::new();
    for token in raw.split_whitespace() {
        if let Ok(code) = token.parse::<i32>() {
            exit_codes.push(code);
        } else if let Some(code) = parse_exit_status_name(token) {
            exit_codes.push(code);
        } else if let Some(sig) = parse_signal_name(token) {
            signals.push(sig);
        } else {
            trace!("SuccessExitStatus: ignoring unrecognised token: {token}");
        }
    }
    SuccessExitStatus {
        exit_codes,
        signals,
    }
}

pub fn parse_service(
    parsed_file: ParsedFile,
    path: &PathBuf,
) -> Result<ParsedServiceConfig, ParsingErrorReason> {
    let mut service_config = None;
    let mut install_config = None;
    let mut unit_config = None;

    for (name, section) in parsed_file {
        match name.as_str() {
            "[Service]" => {
                service_config = Some(parse_service_section(section)?);
            }
            "[Unit]" => {
                unit_config = Some(parse_unit_section(section)?);
            }
            "[Install]" => {
                install_config = Some(parse_install_section(section)?);
            }

            _ if name.starts_with("[X-") || name.starts_with("[x-") => {
                trace!(
                    "Silently ignoring vendor extension section in service unit {path:?}: {name}"
                );
            }
            _ => {
                trace!("Ignoring unknown section in service unit {path:?}: {name}");
            }
        }
    }

    // systemd allows .service files without a [Service] section (e.g.
    // systemd-reboot.service which only has a [Unit] with SuccessAction=).
    // Treat them as oneshot services with no ExecStart.
    let service_config = match service_config {
        Some(c) => c,
        None => {
            trace!(
                "Service unit {path:?} has no [Service] section, treating as oneshot with no ExecStart"
            );
            let empty_section: ParsedSection = HashMap::new();
            parse_service_section(empty_section)?
        }
    };

    Ok(ParsedServiceConfig {
        common: ParsedCommonConfig {
            name: path.file_name().unwrap().to_str().unwrap().to_owned(),
            unit: unit_config.unwrap_or_else(Default::default),
            install: install_config.unwrap_or_else(Default::default),
            fragment_path: Some(path.clone()),
        },
        srvc: service_config,
    })
}

pub(super) fn parse_timeout(descr: &str) -> Timeout {
    if descr.trim().eq_ignore_ascii_case("infinity") {
        Timeout::Infinity
    } else if let Ok(secs) = descr.trim().parse::<u64>() {
        Timeout::Duration(std::time::Duration::from_secs(secs))
    } else if let Some(dur) = crate::units::from_parsed_config::parse_timespan(descr) {
        Timeout::Duration(dur)
    } else {
        // Unparsable value — treat as zero (disabled).
        Timeout::Duration(std::time::Duration::ZERO)
    }
}

pub(super) fn parse_cmdlines(
    raw_lines: &Vec<(u32, String)>,
) -> Result<Vec<Commandline>, ParsingErrorReason> {
    let mut cmdlines = Vec::new();
    for (_line, cmdline) in raw_lines {
        cmdlines.push(parse_cmdline(cmdline)?);
    }
    Ok(cmdlines)
}

pub(crate) fn parse_cmdline(raw_line: &str) -> Result<Commandline, ParsingErrorReason> {
    let mut split = shlex::split(raw_line).ok_or(ParsingErrorReason::Generic(format!(
        "Could not parse cmdline: {raw_line}"
    )))?;
    if split.is_empty() {
        return Err(ParsingErrorReason::Generic(format!(
            "Empty command line: {raw_line}"
        )));
    }
    let mut cmd = split.remove(0);

    let mut prefixes = Vec::new();
    loop {
        let prefix = match &cmd[..1] {
            "-" => {
                cmd = cmd[1..].to_owned();
                CommandlinePrefix::Minus
            }
            "+" => {
                cmd = cmd[1..].to_owned();
                CommandlinePrefix::Plus
            }
            "@" => {
                cmd = cmd[1..].to_owned();
                CommandlinePrefix::AtSign
            }
            ":" => {
                cmd = cmd[1..].to_owned();
                CommandlinePrefix::Colon
            }
            "|" => {
                cmd = cmd[1..].to_owned();
                CommandlinePrefix::Pipe
            }
            "!" => {
                if cmd.len() > 1 && &cmd[1..2] == "!" {
                    cmd = cmd[2..].to_owned();
                    CommandlinePrefix::DoubleExclamation
                } else {
                    cmd = cmd[1..].to_owned();
                    CommandlinePrefix::Exclamation
                }
            }
            _ => break,
        };
        prefixes.push(prefix);
    }
    Ok(Commandline {
        cmd,
        prefixes,
        args: split,
    })
}

fn parse_service_section(
    mut section: ParsedSection,
) -> Result<ParsedServiceSection, ParsingErrorReason> {
    let exec = section.remove("EXECSTART");
    let reload = section.remove("EXECRELOAD");
    let stop = section.remove("EXECSTOP");
    let stoppost = section.remove("EXECSTOPPOST");
    let startpre = section.remove("EXECSTARTPRE");
    let startpost = section.remove("EXECSTARTPOST");
    let starttimeout = section.remove("TIMEOUTSTARTSEC");
    let stoptimeout = section.remove("TIMEOUTSTOPSEC");
    let generaltimeout = section.remove("TIMEOUTSEC");

    let restart = section.remove("RESTART");
    let restart_sec = section.remove("RESTARTSEC");
    let kill_mode = section.remove("KILLMODE");
    let delegate = section.remove("DELEGATE");
    let tasks_max = section.remove("TASKSMAX");
    let limit_nofile = section.remove("LIMITNOFILE");
    let sockets = section.remove("SOCKETS");
    let notify_access = section.remove("NOTIFYACCESS");
    let srcv_type = section.remove("TYPE");
    let accept = section.remove("ACCEPT");
    let dbus_name = section.remove("BUSNAME");
    let pid_file = section.remove("PIDFILE");
    let slice = section.remove("SLICE");
    let remain_after_exit = section.remove("REMAINAFTEREXIT");
    let success_exit_status = section.remove("SUCCESSEXITSTATUS");
    let restart_force_exit_status = section.remove("RESTARTFORCEEXITSTATUS");
    let send_sighup = section.remove("SENDSIGHUP");
    let memory_pressure_watch = section.remove("MEMORYPRESSUREWATCH");
    let reload_signal = section.remove("RELOADSIGNAL");
    let delegate_subgroup = section.remove("DELEGATESUBGROUP");
    let keyring_mode = section.remove("KEYRINGMODE");
    let device_allow = section.remove("DEVICEALLOW");
    let device_policy = section.remove("DEVICEPOLICY");
    let watchdog_sec = section.remove("WATCHDOGSEC");
    let ip_address_allow = section.remove("IPADDRESSALLOW");
    let ip_address_deny = section.remove("IPADDRESSDENY");
    let file_descriptor_store_max = section.remove("FILEDESCRIPTORSTOREMAX");
    let file_descriptor_store_preserve = section.remove("FILEDESCRIPTORSTOREPRESERVE");
    let kill_signal = section.remove("KILLSIGNAL");
    let send_sigkill = section.remove("SENDSIGKILL");
    let restart_kill_signal = section.remove("RESTARTKILLSIGNAL");
    let final_kill_signal = section.remove("FINALKILLSIGNAL");
    let watchdog_signal = section.remove("WATCHDOGSIGNAL");
    let memory_min = section.remove("MEMORYMIN");
    let memory_low = section.remove("MEMORYLOW");
    let memory_high = section.remove("MEMORYHIGH");
    let memory_max = section.remove("MEMORYMAX");
    let memory_swap_max = section.remove("MEMORYSWAPMAX");
    let cpu_weight = section.remove("CPUWEIGHT");
    let startup_cpu_weight = section.remove("STARTUPCPUWEIGHT");
    let cpu_quota = section.remove("CPUQUOTA");
    let io_weight = section.remove("IOWEIGHT");
    let startup_io_weight = section.remove("STARTUPIOWEIGHT");
    let io_device_weight = section.remove("IODEVICEWEIGHT");
    let io_read_bandwidth_max = section.remove("IOREADBANDWIDTHMAX");
    let io_write_bandwidth_max = section.remove("IOWRITEBANDWIDTHMAX");
    let io_read_iops_max = section.remove("IOREADIOPSMAX");
    let io_write_iops_max = section.remove("IOWRITEIOPSMAX");
    let cpu_accounting = section.remove("CPUACCOUNTING");
    let memory_accounting = section.remove("MEMORYACCOUNTING");
    let io_accounting = section.remove("IOACCOUNTING");
    let tasks_accounting = section.remove("TASKSACCOUNTING");
    let runtime_max_sec = section.remove("RUNTIMEMAXSEC");
    let coredump_receive = section.remove("COREDUMPRECEIVE");

    // New service directives
    let exit_type = section.remove("EXITTYPE");
    let oom_policy = section.remove("OOMPOLICY");
    let timeout_abort_sec = section.remove("TIMEOUTABORTSEC");
    let timeout_clean_sec = section.remove("TIMEOUTCLEANSEC");
    let restart_prevent_exit_status = section.remove("RESTARTPREVENTEXITSTATUS");
    let restart_mode = section.remove("RESTARTMODE");
    let restart_steps = section.remove("RESTARTSTEPS");
    let restart_max_delay_sec = section.remove("RESTARTMAXDELAYSEC");
    let exec_condition = section.remove("EXECCONDITION");

    // 9 missing service directives
    let guess_main_pid = section.remove("GUESSMAINPID");
    let timeout_start_failure_mode = section.remove("TIMEOUTSTARTFAILUREMODE");
    let timeout_stop_failure_mode = section.remove("TIMEOUTSTOPFAILUREMODE");
    let runtime_randomized_extra_sec = section.remove("RUNTIMERANDOMIZEDEXTRASEC");
    let root_directory_start_only = section.remove("ROOTDIRECTORYSTARTONLY");
    let non_blocking = section.remove("NONBLOCKING");
    let usb_function_descriptors = section.remove("USBFUNCTIONDESCRIPTORS");
    let usb_function_strings = section.remove("USBFUNCTIONSTRINGS");
    let open_file = section.remove("OPENFILE");

    // 18 missing resource-control directives
    let cpu_quota_period_sec = section.remove("CPUQUOTAPERIODSEC");
    let allowed_cpus = section.remove("ALLOWEDCPUS");
    let startup_allowed_cpus = section.remove("STARTUPALLOWEDCPUS");
    let allowed_memory_nodes = section.remove("ALLOWEDMEMORYNODES");
    let startup_allowed_memory_nodes = section.remove("STARTUPALLOWEDMEMORYNODES");
    let default_memory_min = section.remove("DEFAULTMEMORYMIN");
    let default_memory_low = section.remove("DEFAULTMEMORYLOW");
    let memory_zswap_max = section.remove("MEMORYZSWAPMAX");
    let io_device_latency_target_sec = section.remove("IODEVICELATENCYTARGETSEC");
    let disable_controllers = section.remove("DISABLECONTROLLERS");
    let memory_pressure_threshold_sec = section.remove("MEMORYPRESSURETHRESHOLDSEC");
    let ip_ingress_filter_path = section.remove("IPINGRESSFILTERPATH");
    let ip_egress_filter_path = section.remove("IPEGRESSFILTERPATH");
    let bpf_program = section.remove("BPFPROGRAM");
    let socket_bind_allow = section.remove("SOCKETBINDALLOW");
    let socket_bind_deny = section.remove("SOCKETBINDDENY");
    let restrict_network_interfaces = section.remove("RESTRICTNETWORKINTERFACES");
    let nft_set = section.remove("NFTSET");

    let exec_config = super::parse_exec_section(&mut section)?;

    for key in section.keys() {
        if key.starts_with("X-") {
            trace!("Silently ignoring vendor extension in [Service] section: {key}");
            continue;
        }
        trace!("Ignoring unsupported setting in [Service] section: {key}");
    }

    let limit_nofile = match limit_nofile {
        Some(vec) => {
            if vec.len() == 1 {
                let val = vec[0].1.trim();
                if val.to_uppercase() == "INFINITY" {
                    Some(ResourceLimit {
                        soft: RLimitValue::Infinity,
                        hard: RLimitValue::Infinity,
                    })
                } else if let Some((soft_str, hard_str)) = val.split_once(':') {
                    let soft = if soft_str.trim().to_uppercase() == "INFINITY" {
                        RLimitValue::Infinity
                    } else {
                        RLimitValue::Value(soft_str.trim().parse::<u64>().map_err(|_| {
                            ParsingErrorReason::Generic(format!(
                                "LimitNOFILE soft value is not valid: {soft_str}"
                            ))
                        })?)
                    };
                    let hard = if hard_str.trim().to_uppercase() == "INFINITY" {
                        RLimitValue::Infinity
                    } else {
                        RLimitValue::Value(hard_str.trim().parse::<u64>().map_err(|_| {
                            ParsingErrorReason::Generic(format!(
                                "LimitNOFILE hard value is not valid: {hard_str}"
                            ))
                        })?)
                    };
                    Some(ResourceLimit { soft, hard })
                } else {
                    let num = val.parse::<u64>().map_err(|_| {
                        ParsingErrorReason::Generic(format!(
                            "LimitNOFILE is not a valid value: {val}"
                        ))
                    })?;
                    Some(ResourceLimit {
                        soft: RLimitValue::Value(num),
                        hard: RLimitValue::Value(num),
                    })
                }
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "LimitNOFILE".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => None,
    };

    let tasks_max = match tasks_max {
        Some(vec) => {
            if vec.len() == 1 {
                let val = vec[0].1.trim();
                if val.to_uppercase() == "INFINITY" {
                    Some(TasksMax::Infinity)
                } else if let Some(pct) = val.strip_suffix('%') {
                    let pct_val = pct.trim().parse::<u64>().map_err(|_| {
                        ParsingErrorReason::Generic(format!(
                            "TasksMax percentage is not a valid number: {val}"
                        ))
                    })?;
                    Some(TasksMax::Percent(pct_val))
                } else {
                    let num = val.parse::<u64>().map_err(|_| {
                        ParsingErrorReason::Generic(format!("TasksMax is not a valid value: {val}"))
                    })?;
                    Some(TasksMax::Value(num))
                }
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "TasksMax".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => None,
    };

    let delegate = match delegate {
        Some(vec) => {
            if vec.len() == 1 {
                let val = &vec[0].1;
                if string_to_bool(val) {
                    Delegate::Yes
                } else if val.to_uppercase() == "NO"
                    || val.to_uppercase() == "FALSE"
                    || val == "0"
                    || val.is_empty()
                {
                    Delegate::No
                } else {
                    // Treat as a space-separated list of controller names
                    let controllers: Vec<String> =
                        val.split_whitespace().map(|s| s.to_owned()).collect();
                    if controllers.is_empty() {
                        Delegate::No
                    } else {
                        Delegate::Controllers(controllers)
                    }
                }
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "Delegate".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => Delegate::default(),
    };

    let kill_mode = match kill_mode {
        Some(vec) => {
            if vec.len() == 1 {
                match vec[0].1.to_lowercase().replace('-', "").as_str() {
                    "controlgroup" => KillMode::ControlGroup,
                    "process" => KillMode::Process,
                    "mixed" => KillMode::Mixed,
                    "none" => KillMode::None,
                    name => {
                        return Err(ParsingErrorReason::UnknownSetting(
                            "KillMode".to_owned(),
                            name.to_owned(),
                        ));
                    }
                }
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "KillMode".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => KillMode::default(),
    };

    let restart_sec = match restart_sec {
        Some(vec) => {
            if vec.len() == 1 {
                Some(parse_timeout(&vec[0].1))
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "RestartSec".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => None,
    };

    let watchdog_sec = match watchdog_sec {
        Some(vec) => {
            if vec.len() == 1 {
                let t = parse_timeout(&vec[0].1);
                match t {
                    // A zero duration means "disabled" — store as None
                    Timeout::Duration(d) if d.is_zero() => None,
                    other => Some(other),
                }
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "WatchdogSec".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => None,
    };

    let starttimeout = match starttimeout {
        Some(vec) => {
            if vec.len() == 1 {
                Some(parse_timeout(&vec[0].1))
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "TimeoutStartSec".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => None,
    };
    let stoptimeout = match stoptimeout {
        Some(vec) => {
            if vec.len() == 1 {
                Some(parse_timeout(&vec[0].1))
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "TimeoutStopSec".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => None,
    };
    let generaltimeout = match generaltimeout {
        Some(vec) => {
            if vec.len() == 1 {
                Some(parse_timeout(&vec[0].1))
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "TimeoutSec".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => None,
    };

    // For Type=oneshot, systemd allows multiple ExecStart= lines and runs
    // them sequentially.  For non-oneshot types, only the last ExecStart=
    // line is meaningful (single main process).  We store all entries and
    // let the runtime decide how to use them based on service type.
    let exec = match exec {
        Some(vec) => parse_cmdlines(&vec)?,
        None => Vec::new(),
    };

    let srcv_type = match srcv_type {
        Some(vec) => {
            if vec.len() == 1 {
                match vec[0].1.as_str() {
                    "simple" => ServiceType::Simple,
                    "notify" => ServiceType::Notify,
                    "notify-reload" => ServiceType::NotifyReload,
                    "oneshot" => ServiceType::OneShot,
                    "forking" => ServiceType::Forking,
                    "idle" => ServiceType::Idle,
                    "exec" => ServiceType::Exec,
                    "dbus" => {
                        if cfg!(feature = "dbus_support") {
                            ServiceType::Dbus
                        } else {
                            return Err(ParsingErrorReason::UnsupportedSetting(
                                "Type=dbus".to_owned(),
                            ));
                        }
                    }
                    name => {
                        return Err(ParsingErrorReason::UnknownSetting(
                            "Type".to_owned(),
                            name.to_owned(),
                        ));
                    }
                }
            } else if vec.is_empty() {
                return Err(ParsingErrorReason::MissingSetting("Type".to_owned()));
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "Type".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // When no Type= is specified and there is no ExecStart=, default to
        // oneshot (matches systemd behavior for exec-less service units like
        // systemd-reboot.service).
        None => {
            if exec.is_empty() {
                ServiceType::OneShot
            } else {
                ServiceType::Simple
            }
        }
    };

    let notifyaccess = match notify_access {
        Some(vec) => {
            if vec.len() == 1 {
                match vec[0].1.as_str() {
                    "all" => NotifyKind::All,
                    "main" => NotifyKind::Main,
                    "exec" => NotifyKind::Exec,
                    "none" => NotifyKind::None,
                    name => {
                        return Err(ParsingErrorReason::UnknownSetting(
                            "NotifyAccess".to_owned(),
                            name.to_owned(),
                        ));
                    }
                }
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "NotifyAccess".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => NotifyKind::Main,
    };

    let reload = match reload {
        Some(vec) => parse_cmdlines(&vec)?,
        None => Vec::new(),
    };
    let stop = match stop {
        Some(vec) => parse_cmdlines(&vec)?,
        None => Vec::new(),
    };
    let stoppost = match stoppost {
        Some(vec) => parse_cmdlines(&vec)?,
        None => Vec::new(),
    };
    let startpre = match startpre {
        Some(vec) => parse_cmdlines(&vec)?,
        None => Vec::new(),
    };
    let startpost = match startpost {
        Some(vec) => parse_cmdlines(&vec)?,
        None => Vec::new(),
    };

    let restart = match restart {
        Some(vec) => {
            if vec.len() == 1 {
                match vec[0].1.to_uppercase().replace('-', "").as_str() {
                    "ALWAYS" => ServiceRestart::Always,
                    "NO" => ServiceRestart::No,
                    "ONSUCCESS" => ServiceRestart::OnSuccess,
                    "ONFAILURE" => ServiceRestart::OnFailure,
                    "ONABNORMAL" => ServiceRestart::OnAbnormal,
                    "ONABORT" => ServiceRestart::OnAbort,
                    "ONWATCHDOG" => ServiceRestart::OnWatchdog,

                    name => {
                        return Err(ParsingErrorReason::UnknownSetting(
                            "Restart".to_owned(),
                            name.to_owned(),
                        ));
                    }
                }
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "Restart".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => ServiceRestart::No,
    };
    let accept = match accept {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "Accept".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => false,
    };
    let dbus_name = match dbus_name {
        Some(vec) => {
            if vec.len() == 1 {
                Some(vec[0].1.clone())
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "BusName".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => None,
    };

    if srcv_type == ServiceType::Dbus && dbus_name.is_none() {
        return Err(ParsingErrorReason::MissingSetting("BusName".to_owned()));
    }

    let pid_file = match pid_file {
        None => None,
        Some(mut vec) => {
            if vec.len() == 1 {
                Some(std::path::PathBuf::from(vec.remove(0).1))
            } else if vec.len() > 1 {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "PIDFile".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            } else {
                None
            }
        }
    };

    let send_sighup = match send_sighup {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "SendSIGHUP".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => false,
    };

    Ok(ParsedServiceSection {
        srcv_type,
        notifyaccess,
        restart,
        restart_sec,
        kill_mode,
        delegate,
        tasks_max,
        limit_nofile,
        accept,
        dbus_name,
        pid_file,
        exec,
        reload,
        stop,
        stoppost,
        startpre,
        startpost,
        starttimeout,
        stoptimeout,
        generaltimeout,
        send_sighup,
        memory_pressure_watch: match memory_pressure_watch {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.to_lowercase().as_str() {
                        "auto" | "" => MemoryPressureWatch::Auto,
                        "on" | "yes" | "true" | "1" => MemoryPressureWatch::On,
                        "off" | "no" | "false" | "0" => MemoryPressureWatch::Off,
                        "skip" => MemoryPressureWatch::Skip,
                        other => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "MemoryPressureWatch".to_owned(),
                                other.to_owned(),
                            ));
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "MemoryPressureWatch".to_owned(),
                        map_tuples_to_second(vec),
                    ));
                }
            }
            None => MemoryPressureWatch::default(),
        },
        sockets: map_tuples_to_second(super::split_list_values(sockets.unwrap_or_default())),
        slice: slice.and_then(|vec| {
            if vec.len() == 1 {
                Some(vec[0].1.clone())
            } else {
                None
            }
        }),
        remain_after_exit: remain_after_exit
            .map(|vec| {
                if vec.len() == 1 {
                    string_to_bool(&vec[0].1)
                } else {
                    false
                }
            })
            .unwrap_or(false),
        success_exit_status: success_exit_status
            .map(|vec| {
                // Merge all entries — systemd allows multiple SuccessExitStatus=
                // lines and they accumulate.
                let combined: Vec<String> = vec.into_iter().map(|(_, v)| v).collect();
                parse_success_exit_status(&combined.join(" "))
            })
            .unwrap_or_default(),
        restart_force_exit_status: restart_force_exit_status
            .map(|vec| {
                // Merge all entries — systemd allows multiple RestartForceExitStatus=
                // lines and they accumulate.
                let combined: Vec<String> = vec.into_iter().map(|(_, v)| v).collect();
                parse_success_exit_status(&combined.join(" "))
            })
            .unwrap_or_default(),
        reload_signal: match reload_signal {
            Some(vec) => {
                if vec.len() == 1 {
                    let raw = vec[0].1.trim();
                    if raw.is_empty() {
                        None
                    } else {
                        match parse_signal_to_raw(raw) {
                            Some(num) => Some(num),
                            None => {
                                return Err(ParsingErrorReason::UnknownSetting(
                                    "ReloadSignal".to_owned(),
                                    raw.to_owned(),
                                ));
                            }
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "ReloadSignal".to_owned(),
                        map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        kill_signal: match kill_signal {
            Some(vec) => {
                if vec.len() == 1 {
                    let raw = vec[0].1.trim();
                    if raw.is_empty() {
                        None
                    } else {
                        match parse_signal_to_raw(raw) {
                            Some(num) => Some(num),
                            None => {
                                return Err(ParsingErrorReason::UnknownSetting(
                                    "KillSignal".to_owned(),
                                    raw.to_owned(),
                                ));
                            }
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "KillSignal".to_owned(),
                        map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        delegate_subgroup: match delegate_subgroup {
            Some(vec) => {
                if vec.len() == 1 {
                    let raw = vec[0].1.trim();
                    if raw.is_empty() {
                        None
                    } else {
                        Some(raw.to_owned())
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "DelegateSubgroup".to_owned(),
                        map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        keyring_mode: match keyring_mode {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.trim().to_lowercase().as_str() {
                        "inherit" => KeyringMode::Inherit,
                        "private" => KeyringMode::Private,
                        "shared" => KeyringMode::Shared,
                        other => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "KeyringMode".to_owned(),
                                other.to_owned(),
                            ));
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "KeyringMode".to_owned(),
                        map_tuples_to_second(vec),
                    ));
                }
            }
            None => KeyringMode::default(),
        },
        device_allow: match device_allow {
            Some(vec) => {
                // Each directive is a device node path (or class like "char-*",
                // "block-*") followed by optional access characters (r, w, m).
                // Multiple directives accumulate. An empty assignment resets
                // the list.
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        // Empty string resets the list
                        entries.clear();
                        continue;
                    }
                    entries.push(trimmed.to_owned());
                }
                entries
            }
            None => Vec::new(),
        },
        device_policy: match device_policy {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.trim().to_lowercase().as_str() {
                        "auto" | "" => super::DevicePolicy::Auto,
                        "closed" => super::DevicePolicy::Closed,
                        "strict" => super::DevicePolicy::Strict,
                        other => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "DevicePolicy".to_owned(),
                                other.to_owned(),
                            ));
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "DevicePolicy".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => super::DevicePolicy::default(),
        },
        watchdog_sec,
        ip_address_allow: match ip_address_allow {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        // Empty string resets the list
                        entries.clear();
                        continue;
                    }
                    // Each directive may contain space-separated addresses
                    for addr in trimmed.split_whitespace() {
                        entries.push(addr.to_owned());
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        ip_address_deny: match ip_address_deny {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        // Empty string resets the list
                        entries.clear();
                        continue;
                    }
                    // Each directive may contain space-separated addresses
                    for addr in trimmed.split_whitespace() {
                        entries.push(addr.to_owned());
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        file_descriptor_store_max: match file_descriptor_store_max {
            Some(vec) => {
                if vec.len() == 1 {
                    let val = vec[0].1.trim();
                    if val.is_empty() {
                        0
                    } else {
                        val.parse::<u64>().map_err(|_| {
                            ParsingErrorReason::Generic(format!(
                                "FileDescriptorStoreMax is not a valid non-negative integer: {val}"
                            ))
                        })?
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "FileDescriptorStoreMax".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => 0,
        },
        file_descriptor_store_preserve: match file_descriptor_store_preserve {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.trim().to_lowercase().as_str() {
                        "no" | "false" | "0" => super::FileDescriptorStorePreserve::No,
                        "yes" | "true" | "1" => super::FileDescriptorStorePreserve::Yes,
                        "restart" => super::FileDescriptorStorePreserve::Restart,
                        other => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "FileDescriptorStorePreserve".to_owned(),
                                other.to_owned(),
                            ));
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "FileDescriptorStorePreserve".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => super::FileDescriptorStorePreserve::default(),
        },
        memory_min: match memory_min {
            Some(vec) => {
                if vec.len() == 1 {
                    parse_memory_limit(&vec[0].1)
                        .map_err(|e| ParsingErrorReason::Generic(format!("MemoryMin: {e}")))?
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "MemoryMin".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        memory_low: match memory_low {
            Some(vec) => {
                if vec.len() == 1 {
                    parse_memory_limit(&vec[0].1)
                        .map_err(|e| ParsingErrorReason::Generic(format!("MemoryLow: {e}")))?
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "MemoryLow".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        memory_high: match memory_high {
            Some(vec) => {
                if vec.len() == 1 {
                    parse_memory_limit(&vec[0].1)
                        .map_err(|e| ParsingErrorReason::Generic(format!("MemoryHigh: {e}")))?
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "MemoryHigh".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        memory_max: match memory_max {
            Some(vec) => {
                if vec.len() == 1 {
                    parse_memory_limit(&vec[0].1)
                        .map_err(|e| ParsingErrorReason::Generic(format!("MemoryMax: {e}")))?
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "MemoryMax".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        memory_swap_max: match memory_swap_max {
            Some(vec) => {
                if vec.len() == 1 {
                    parse_memory_limit(&vec[0].1)
                        .map_err(|e| ParsingErrorReason::Generic(format!("MemorySwapMax: {e}")))?
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "MemorySwapMax".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        cpu_weight: match cpu_weight {
            Some(vec) => {
                if vec.len() == 1 {
                    parse_cpu_weight(&vec[0].1)
                        .map_err(|e| ParsingErrorReason::Generic(format!("CPUWeight: {e}")))?
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "CPUWeight".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        startup_cpu_weight: match startup_cpu_weight {
            Some(vec) => {
                if vec.len() == 1 {
                    parse_cpu_weight(&vec[0].1).map_err(|e| {
                        ParsingErrorReason::Generic(format!("StartupCPUWeight: {e}"))
                    })?
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "StartupCPUWeight".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        cpu_quota: match cpu_quota {
            Some(vec) => {
                if vec.len() == 1 {
                    parse_cpu_quota(&vec[0].1)
                        .map_err(|e| ParsingErrorReason::Generic(format!("CPUQuota: {e}")))?
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "CPUQuota".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        io_weight: match io_weight {
            Some(vec) => {
                if vec.len() == 1 {
                    parse_io_weight(&vec[0].1)
                        .map_err(|e| ParsingErrorReason::Generic(format!("IOWeight: {e}")))?
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "IOWeight".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        startup_io_weight: match startup_io_weight {
            Some(vec) => {
                if vec.len() == 1 {
                    parse_io_weight(&vec[0].1)
                        .map_err(|e| ParsingErrorReason::Generic(format!("StartupIOWeight: {e}")))?
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "StartupIOWeight".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        io_device_weight: match io_device_weight {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    if let Some(limit) = parse_io_device_limit(trimmed)
                        .map_err(|e| ParsingErrorReason::Generic(format!("IODeviceWeight: {e}")))?
                    {
                        entries.push(limit);
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        io_read_bandwidth_max: match io_read_bandwidth_max {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    if let Some(limit) = parse_io_device_limit(trimmed).map_err(|e| {
                        ParsingErrorReason::Generic(format!("IOReadBandwidthMax: {e}"))
                    })? {
                        entries.push(limit);
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        io_write_bandwidth_max: match io_write_bandwidth_max {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    if let Some(limit) = parse_io_device_limit(trimmed).map_err(|e| {
                        ParsingErrorReason::Generic(format!("IOWriteBandwidthMax: {e}"))
                    })? {
                        entries.push(limit);
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        io_read_iops_max: match io_read_iops_max {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    if let Some(limit) = parse_io_device_limit(trimmed)
                        .map_err(|e| ParsingErrorReason::Generic(format!("IOReadIOPSMax: {e}")))?
                    {
                        entries.push(limit);
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        io_write_iops_max: match io_write_iops_max {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    if let Some(limit) = parse_io_device_limit(trimmed)
                        .map_err(|e| ParsingErrorReason::Generic(format!("IOWriteIOPSMax: {e}")))?
                    {
                        entries.push(limit);
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        cpu_accounting: match cpu_accounting {
            Some(vec) => {
                if vec.len() == 1 {
                    Some(string_to_bool(&vec[0].1))
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "CPUAccounting".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        memory_accounting: match memory_accounting {
            Some(vec) => {
                if vec.len() == 1 {
                    Some(string_to_bool(&vec[0].1))
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "MemoryAccounting".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        io_accounting: match io_accounting {
            Some(vec) => {
                if vec.len() == 1 {
                    Some(string_to_bool(&vec[0].1))
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "IOAccounting".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        tasks_accounting: match tasks_accounting {
            Some(vec) => {
                if vec.len() == 1 {
                    Some(string_to_bool(&vec[0].1))
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "TasksAccounting".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        runtime_max_sec: match runtime_max_sec {
            Some(vec) => {
                if vec.len() == 1 {
                    let t = parse_timeout(&vec[0].1);
                    match t {
                        // A zero duration means "no limit" — store as None
                        Timeout::Duration(d) if d.is_zero() => None,
                        // Infinity means "no limit" — store as None
                        Timeout::Infinity => None,
                        other => Some(other),
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "RuntimeMaxSec".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        coredump_receive: coredump_receive
            .map(|vec| {
                if vec.len() == 1 {
                    string_to_bool(&vec[0].1)
                } else {
                    false
                }
            })
            .unwrap_or(false),
        send_sigkill: send_sigkill
            .map(|vec| {
                if vec.len() == 1 {
                    string_to_bool(&vec[0].1)
                } else {
                    true // default
                }
            })
            .unwrap_or(true), // SendSIGKILL= defaults to true
        restart_kill_signal: match restart_kill_signal {
            Some(vec) => {
                if vec.len() == 1 {
                    parse_signal_to_raw(&vec[0].1)
                } else {
                    None
                }
            }
            None => None,
        },
        final_kill_signal: match final_kill_signal {
            Some(vec) => {
                if vec.len() == 1 {
                    parse_signal_to_raw(&vec[0].1)
                } else {
                    None
                }
            }
            None => None,
        },
        watchdog_signal: match watchdog_signal {
            Some(vec) => {
                if vec.len() == 1 {
                    parse_signal_to_raw(&vec[0].1)
                } else {
                    None
                }
            }
            None => None,
        },
        exit_type: match exit_type {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.trim().to_lowercase().as_str() {
                        "main" | "" => super::ExitType::Main,
                        "cgroup" => super::ExitType::Cgroup,
                        other => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "ExitType".to_owned(),
                                other.to_owned(),
                            ));
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "ExitType".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => super::ExitType::default(),
        },
        oom_policy: match oom_policy {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.trim().to_lowercase().as_str() {
                        "continue" => super::OOMPolicy::Continue,
                        "stop" | "" => super::OOMPolicy::Stop,
                        "kill" => super::OOMPolicy::Kill,
                        other => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "OOMPolicy".to_owned(),
                                other.to_owned(),
                            ));
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "OOMPolicy".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => super::OOMPolicy::default(),
        },
        timeout_abort_sec: match timeout_abort_sec {
            Some(vec) => {
                if vec.len() == 1 {
                    let val = vec[0].1.trim();
                    if val.is_empty() {
                        None
                    } else {
                        Some(parse_timeout(val))
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "TimeoutAbortSec".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        timeout_clean_sec: match timeout_clean_sec {
            Some(vec) => {
                if vec.len() == 1 {
                    let val = vec[0].1.trim();
                    if val.is_empty() {
                        None
                    } else {
                        Some(parse_timeout(val))
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "TimeoutCleanSec".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        restart_prevent_exit_status: restart_prevent_exit_status
            .map(|vec| {
                let combined: Vec<String> = vec.into_iter().map(|(_, v)| v).collect();
                parse_success_exit_status(&combined.join(" "))
            })
            .unwrap_or_default(),
        restart_mode: match restart_mode {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.trim().to_lowercase().as_str() {
                        "direct" | "" => super::RestartMode::Direct,
                        "normal" => super::RestartMode::Normal,
                        other => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "RestartMode".to_owned(),
                                other.to_owned(),
                            ));
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "RestartMode".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => super::RestartMode::default(),
        },
        restart_steps: match restart_steps {
            Some(vec) => {
                if vec.len() == 1 {
                    let val = vec[0].1.trim();
                    if val.is_empty() {
                        0
                    } else {
                        val.parse::<u32>().map_err(|_| {
                            ParsingErrorReason::Generic(format!(
                                "RestartSteps is not a valid non-negative integer: {val}"
                            ))
                        })?
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "RestartSteps".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => 0,
        },
        restart_max_delay_sec: match restart_max_delay_sec {
            Some(vec) => {
                if vec.len() == 1 {
                    let t = parse_timeout(&vec[0].1);
                    match t {
                        Timeout::Infinity => None,
                        Timeout::Duration(d) if d.is_zero() => None,
                        other => Some(other),
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "RestartMaxDelaySec".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        exec_condition: match exec_condition {
            Some(vec) => parse_cmdlines(&vec)?,
            None => Vec::new(),
        },

        // 9 new service directives
        guess_main_pid: guess_main_pid
            .map(|vec| {
                if vec.len() == 1 {
                    string_to_bool(&vec[0].1)
                } else {
                    true // default
                }
            })
            .unwrap_or(true), // GuessMainPID= defaults to true
        timeout_start_failure_mode: match timeout_start_failure_mode {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.trim().to_lowercase().as_str() {
                        "terminate" | "" => super::TimeoutFailureMode::Terminate,
                        "abort" => super::TimeoutFailureMode::Abort,
                        other => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "TimeoutStartFailureMode".to_owned(),
                                other.to_owned(),
                            ));
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "TimeoutStartFailureMode".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => super::TimeoutFailureMode::default(),
        },
        timeout_stop_failure_mode: match timeout_stop_failure_mode {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.trim().to_lowercase().as_str() {
                        "terminate" | "" => super::TimeoutFailureMode::Terminate,
                        "abort" => super::TimeoutFailureMode::Abort,
                        other => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "TimeoutStopFailureMode".to_owned(),
                                other.to_owned(),
                            ));
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "TimeoutStopFailureMode".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => super::TimeoutFailureMode::default(),
        },
        runtime_randomized_extra_sec: match runtime_randomized_extra_sec {
            Some(vec) => {
                if vec.len() == 1 {
                    let val = vec[0].1.trim();
                    if val.is_empty() {
                        None
                    } else {
                        let t = parse_timeout(val);
                        match t {
                            Timeout::Duration(d) if d.is_zero() => None,
                            other => Some(other),
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "RuntimeRandomizedExtraSec".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        root_directory_start_only: root_directory_start_only
            .map(|vec| {
                if vec.len() == 1 {
                    string_to_bool(&vec[0].1)
                } else {
                    false
                }
            })
            .unwrap_or(false),
        non_blocking: non_blocking
            .map(|vec| {
                if vec.len() == 1 {
                    string_to_bool(&vec[0].1)
                } else {
                    false
                }
            })
            .unwrap_or(false),
        usb_function_descriptors: match usb_function_descriptors {
            Some(vec) => {
                if vec.len() == 1 {
                    let val = vec[0].1.trim();
                    if val.is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(val))
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "USBFunctionDescriptors".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        usb_function_strings: match usb_function_strings {
            Some(vec) => {
                if vec.len() == 1 {
                    let val = vec[0].1.trim();
                    if val.is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(val))
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "USBFunctionStrings".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        open_file: match open_file {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    entries.push(trimmed.to_owned());
                }
                entries
            }
            None => Vec::new(),
        },

        // 18 new resource-control directives
        cpu_quota_period_sec: match cpu_quota_period_sec {
            Some(vec) => {
                if vec.len() == 1 {
                    let val = vec[0].1.trim();
                    if val.is_empty() {
                        None
                    } else {
                        Some(parse_timeout(val))
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "CPUQuotaPeriodSec".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        allowed_cpus: match allowed_cpus {
            Some(vec) => {
                if vec.len() == 1 {
                    let val = vec[0].1.trim();
                    if val.is_empty() {
                        None
                    } else {
                        Some(val.to_owned())
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "AllowedCPUs".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        startup_allowed_cpus: match startup_allowed_cpus {
            Some(vec) => {
                if vec.len() == 1 {
                    let val = vec[0].1.trim();
                    if val.is_empty() {
                        None
                    } else {
                        Some(val.to_owned())
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "StartupAllowedCPUs".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        allowed_memory_nodes: match allowed_memory_nodes {
            Some(vec) => {
                if vec.len() == 1 {
                    let val = vec[0].1.trim();
                    if val.is_empty() {
                        None
                    } else {
                        Some(val.to_owned())
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "AllowedMemoryNodes".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        startup_allowed_memory_nodes: match startup_allowed_memory_nodes {
            Some(vec) => {
                if vec.len() == 1 {
                    let val = vec[0].1.trim();
                    if val.is_empty() {
                        None
                    } else {
                        Some(val.to_owned())
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "StartupAllowedMemoryNodes".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        default_memory_min: match default_memory_min {
            Some(vec) => {
                if vec.len() == 1 {
                    super::parse_memory_limit(vec[0].1.trim()).map_err(|e| {
                        ParsingErrorReason::Generic(format!("DefaultMemoryMin: {e}"))
                    })?
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "DefaultMemoryMin".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        default_memory_low: match default_memory_low {
            Some(vec) => {
                if vec.len() == 1 {
                    super::parse_memory_limit(vec[0].1.trim()).map_err(|e| {
                        ParsingErrorReason::Generic(format!("DefaultMemoryLow: {e}"))
                    })?
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "DefaultMemoryLow".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        memory_zswap_max: match memory_zswap_max {
            Some(vec) => {
                if vec.len() == 1 {
                    super::parse_memory_limit(vec[0].1.trim())
                        .map_err(|e| ParsingErrorReason::Generic(format!("MemoryZSwapMax: {e}")))?
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "MemoryZSwapMax".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        io_device_latency_target_sec: match io_device_latency_target_sec {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    entries.push(trimmed.to_owned());
                }
                entries
            }
            None => Vec::new(),
        },
        disable_controllers: match disable_controllers {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    for token in trimmed.split_whitespace() {
                        entries.push(token.to_owned());
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        memory_pressure_threshold_sec: match memory_pressure_threshold_sec {
            Some(vec) => {
                if vec.len() == 1 {
                    let val = vec[0].1.trim();
                    if val.is_empty() {
                        None
                    } else {
                        Some(parse_timeout(val))
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "MemoryPressureThresholdSec".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => None,
        },
        ip_ingress_filter_path: match ip_ingress_filter_path {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    entries.push(trimmed.to_owned());
                }
                entries
            }
            None => Vec::new(),
        },
        ip_egress_filter_path: match ip_egress_filter_path {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    entries.push(trimmed.to_owned());
                }
                entries
            }
            None => Vec::new(),
        },
        bpf_program: match bpf_program {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    entries.push(trimmed.to_owned());
                }
                entries
            }
            None => Vec::new(),
        },
        socket_bind_allow: match socket_bind_allow {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    entries.push(trimmed.to_owned());
                }
                entries
            }
            None => Vec::new(),
        },
        socket_bind_deny: match socket_bind_deny {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    entries.push(trimmed.to_owned());
                }
                entries
            }
            None => Vec::new(),
        },
        restrict_network_interfaces: match restrict_network_interfaces {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    for token in trimmed.split_whitespace() {
                        entries.push(token.to_owned());
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        nft_set: match nft_set {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    entries.push(trimmed.to_owned());
                }
                entries
            }
            None => Vec::new(),
        },

        exec_section: exec_config,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::unit_parsing::unit_parser::parse_file;

    fn parse_service_from_str(content: &str) -> Result<ParsedServiceConfig, ParsingErrorReason> {
        let path = PathBuf::from("/test/test.service");
        let parsed_file = parse_file(content)?;
        parse_service(parsed_file, &path)
    }

    // --- ExitType= ---

    #[test]
    fn test_exit_type_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.exit_type, super::super::ExitType::Main);
    }

    #[test]
    fn test_exit_type_main() {
        let content = "[Service]\nExecStart=/bin/true\nExitType=main\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.exit_type, super::super::ExitType::Main);
    }

    #[test]
    fn test_exit_type_cgroup() {
        let content = "[Service]\nExecStart=/bin/true\nExitType=cgroup\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.exit_type, super::super::ExitType::Cgroup);
    }

    #[test]
    fn test_exit_type_invalid() {
        let content = "[Service]\nExecStart=/bin/true\nExitType=bogus\n";
        let result = parse_service_from_str(content);
        assert!(result.is_err());
    }

    // --- OOMPolicy= ---

    #[test]
    fn test_oom_policy_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.oom_policy, super::super::OOMPolicy::Stop);
    }

    #[test]
    fn test_oom_policy_continue() {
        let content = "[Service]\nExecStart=/bin/true\nOOMPolicy=continue\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.oom_policy, super::super::OOMPolicy::Continue);
    }

    #[test]
    fn test_oom_policy_stop() {
        let content = "[Service]\nExecStart=/bin/true\nOOMPolicy=stop\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.oom_policy, super::super::OOMPolicy::Stop);
    }

    #[test]
    fn test_oom_policy_kill() {
        let content = "[Service]\nExecStart=/bin/true\nOOMPolicy=kill\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.oom_policy, super::super::OOMPolicy::Kill);
    }

    #[test]
    fn test_oom_policy_invalid() {
        let content = "[Service]\nExecStart=/bin/true\nOOMPolicy=bogus\n";
        let result = parse_service_from_str(content);
        assert!(result.is_err());
    }

    // --- TimeoutAbortSec= ---

    #[test]
    fn test_timeout_abort_sec_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.timeout_abort_sec.is_none());
    }

    #[test]
    fn test_timeout_abort_sec_value() {
        let content = "[Service]\nExecStart=/bin/true\nTimeoutAbortSec=30\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.timeout_abort_sec,
            Some(Timeout::Duration(std::time::Duration::from_secs(30)))
        );
    }

    #[test]
    fn test_timeout_abort_sec_infinity() {
        let content = "[Service]\nExecStart=/bin/true\nTimeoutAbortSec=infinity\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.timeout_abort_sec, Some(Timeout::Infinity));
    }

    #[test]
    fn test_timeout_abort_sec_empty() {
        let content = "[Service]\nExecStart=/bin/true\nTimeoutAbortSec=\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.timeout_abort_sec.is_none());
    }

    // --- TimeoutCleanSec= ---

    #[test]
    fn test_timeout_clean_sec_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.timeout_clean_sec.is_none());
    }

    #[test]
    fn test_timeout_clean_sec_value() {
        let content = "[Service]\nExecStart=/bin/true\nTimeoutCleanSec=60\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.timeout_clean_sec,
            Some(Timeout::Duration(std::time::Duration::from_secs(60)))
        );
    }

    #[test]
    fn test_timeout_clean_sec_infinity() {
        let content = "[Service]\nExecStart=/bin/true\nTimeoutCleanSec=infinity\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.timeout_clean_sec, Some(Timeout::Infinity));
    }

    // --- RestartPreventExitStatus= ---

    #[test]
    fn test_restart_prevent_exit_status_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(
            config
                .srvc
                .restart_prevent_exit_status
                .exit_codes
                .is_empty()
        );
        assert!(config.srvc.restart_prevent_exit_status.signals.is_empty());
    }

    #[test]
    fn test_restart_prevent_exit_status_codes() {
        let content = "[Service]\nExecStart=/bin/true\nRestartPreventExitStatus=1 6 SIGTERM\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(
            config
                .srvc
                .restart_prevent_exit_status
                .exit_codes
                .contains(&1)
        );
        assert!(
            config
                .srvc
                .restart_prevent_exit_status
                .exit_codes
                .contains(&6)
        );
        assert!(
            config
                .srvc
                .restart_prevent_exit_status
                .signals
                .contains(&nix::sys::signal::Signal::SIGTERM)
        );
    }

    #[test]
    fn test_restart_prevent_exit_status_signal_only() {
        let content = "[Service]\nExecStart=/bin/true\nRestartPreventExitStatus=SIGHUP\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(
            config
                .srvc
                .restart_prevent_exit_status
                .exit_codes
                .is_empty()
        );
        assert!(
            config
                .srvc
                .restart_prevent_exit_status
                .signals
                .contains(&nix::sys::signal::Signal::SIGHUP)
        );
    }

    // --- RestartMode= ---

    #[test]
    fn test_restart_mode_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.restart_mode, super::super::RestartMode::Direct);
    }

    #[test]
    fn test_restart_mode_direct() {
        let content = "[Service]\nExecStart=/bin/true\nRestartMode=direct\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.restart_mode, super::super::RestartMode::Direct);
    }

    #[test]
    fn test_restart_mode_normal() {
        let content = "[Service]\nExecStart=/bin/true\nRestartMode=normal\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.restart_mode, super::super::RestartMode::Normal);
    }

    #[test]
    fn test_restart_mode_invalid() {
        let content = "[Service]\nExecStart=/bin/true\nRestartMode=bogus\n";
        let result = parse_service_from_str(content);
        assert!(result.is_err());
    }

    // --- RestartSteps= ---

    #[test]
    fn test_restart_steps_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.restart_steps, 0);
    }

    #[test]
    fn test_restart_steps_value() {
        let content = "[Service]\nExecStart=/bin/true\nRestartSteps=5\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.restart_steps, 5);
    }

    #[test]
    fn test_restart_steps_zero() {
        let content = "[Service]\nExecStart=/bin/true\nRestartSteps=0\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.restart_steps, 0);
    }

    #[test]
    fn test_restart_steps_empty() {
        let content = "[Service]\nExecStart=/bin/true\nRestartSteps=\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.restart_steps, 0);
    }

    #[test]
    fn test_restart_steps_invalid() {
        let content = "[Service]\nExecStart=/bin/true\nRestartSteps=abc\n";
        let result = parse_service_from_str(content);
        assert!(result.is_err());
    }

    // --- RestartMaxDelaySec= ---

    #[test]
    fn test_restart_max_delay_sec_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.restart_max_delay_sec.is_none());
    }

    #[test]
    fn test_restart_max_delay_sec_value() {
        let content = "[Service]\nExecStart=/bin/true\nRestartMaxDelaySec=120\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.restart_max_delay_sec,
            Some(Timeout::Duration(std::time::Duration::from_secs(120)))
        );
    }

    #[test]
    fn test_restart_max_delay_sec_infinity() {
        // infinity means no upper bound — stored as None
        let content = "[Service]\nExecStart=/bin/true\nRestartMaxDelaySec=infinity\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.restart_max_delay_sec.is_none());
    }

    #[test]
    fn test_restart_max_delay_sec_zero() {
        // 0 means no upper bound — stored as None
        let content = "[Service]\nExecStart=/bin/true\nRestartMaxDelaySec=0\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.restart_max_delay_sec.is_none());
    }

    // --- ExecCondition= ---

    #[test]
    fn test_exec_condition_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.exec_condition.is_empty());
    }

    #[test]
    fn test_exec_condition_single() {
        let content = "[Service]\nExecStart=/bin/true\nExecCondition=/usr/bin/test -f /tmp/ready\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.exec_condition.len(), 1);
        assert_eq!(config.srvc.exec_condition[0].cmd, "/usr/bin/test");
    }

    #[test]
    fn test_exec_condition_multiple() {
        let content = "[Service]\nExecStart=/bin/true\nExecCondition=/usr/bin/test -f /tmp/a\nExecCondition=/usr/bin/test -f /tmp/b\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.exec_condition.len(), 2);
    }

    #[test]
    fn test_exec_condition_with_dash_prefix() {
        let content =
            "[Service]\nExecStart=/bin/true\nExecCondition=-/usr/bin/test -f /tmp/ready\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.exec_condition.len(), 1);
        assert!(
            config.srvc.exec_condition[0]
                .prefixes
                .contains(&CommandlinePrefix::Minus)
        );
    }

    #[test]
    fn test_exec_condition_empty_is_error() {
        // An empty ExecCondition= value is treated as an invalid (empty)
        // command line by parse_cmdlines, not as a list reset.
        let content = "[Service]\nExecStart=/bin/true\nExecCondition=/usr/bin/test -f /tmp/a\nExecCondition=\n";
        let result = parse_service_from_str(content);
        assert!(result.is_err());
    }

    // --- Combined / integration tests ---

    #[test]
    fn test_all_new_directives_together() {
        let content = "\
[Service]
ExecStart=/bin/myapp
ExitType=cgroup
OOMPolicy=continue
TimeoutAbortSec=45
TimeoutCleanSec=30
RestartPreventExitStatus=42 SIGKILL
RestartMode=direct
RestartSteps=10
RestartMaxDelaySec=300
ExecCondition=/usr/bin/test -f /etc/ready
";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.exit_type, super::super::ExitType::Cgroup);
        assert_eq!(config.srvc.oom_policy, super::super::OOMPolicy::Continue);
        assert_eq!(
            config.srvc.timeout_abort_sec,
            Some(Timeout::Duration(std::time::Duration::from_secs(45)))
        );
        assert_eq!(
            config.srvc.timeout_clean_sec,
            Some(Timeout::Duration(std::time::Duration::from_secs(30)))
        );
        assert!(
            config
                .srvc
                .restart_prevent_exit_status
                .exit_codes
                .contains(&42)
        );
        assert!(
            config
                .srvc
                .restart_prevent_exit_status
                .signals
                .contains(&nix::sys::signal::Signal::SIGKILL)
        );
        assert_eq!(config.srvc.restart_mode, super::super::RestartMode::Direct);
        assert_eq!(config.srvc.restart_steps, 10);
        assert_eq!(
            config.srvc.restart_max_delay_sec,
            Some(Timeout::Duration(std::time::Duration::from_secs(300)))
        );
        assert_eq!(config.srvc.exec_condition.len(), 1);
    }

    #[test]
    fn test_no_service_section_defaults() {
        // A .service file with only [Unit] should get default values for all new fields
        let content = "[Unit]\nDescription=Test\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.exit_type, super::super::ExitType::Main);
        assert_eq!(config.srvc.oom_policy, super::super::OOMPolicy::Stop);
        assert!(config.srvc.timeout_abort_sec.is_none());
        assert!(config.srvc.timeout_clean_sec.is_none());
        assert!(
            config
                .srvc
                .restart_prevent_exit_status
                .exit_codes
                .is_empty()
        );
        assert_eq!(config.srvc.restart_mode, super::super::RestartMode::Direct);
        assert_eq!(config.srvc.restart_steps, 0);
        assert!(config.srvc.restart_max_delay_sec.is_none());
        assert!(config.srvc.exec_condition.is_empty());
    }

    #[test]
    fn test_exit_type_case_insensitive() {
        let content = "[Service]\nExecStart=/bin/true\nExitType=CGROUP\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.exit_type, super::super::ExitType::Cgroup);
    }

    #[test]
    fn test_oom_policy_case_insensitive() {
        let content = "[Service]\nExecStart=/bin/true\nOOMPolicy=CONTINUE\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.oom_policy, super::super::OOMPolicy::Continue);
    }

    #[test]
    fn test_restart_mode_case_insensitive() {
        let content = "[Service]\nExecStart=/bin/true\nRestartMode=NORMAL\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.restart_mode, super::super::RestartMode::Normal);
    }

    // =====================================================================
    // 9 new service directives
    // =====================================================================

    // --- GuessMainPID= ---

    #[test]
    fn test_guess_main_pid_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.guess_main_pid, "GuessMainPID defaults to true");
    }

    #[test]
    fn test_guess_main_pid_yes() {
        let content = "[Service]\nExecStart=/bin/true\nGuessMainPID=yes\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.guess_main_pid);
    }

    #[test]
    fn test_guess_main_pid_no() {
        let content = "[Service]\nExecStart=/bin/true\nGuessMainPID=no\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(!config.srvc.guess_main_pid);
    }

    #[test]
    fn test_guess_main_pid_false() {
        let content = "[Service]\nExecStart=/bin/true\nGuessMainPID=false\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(!config.srvc.guess_main_pid);
    }

    // --- TimeoutStartFailureMode= ---

    #[test]
    fn test_timeout_start_failure_mode_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.timeout_start_failure_mode,
            super::super::TimeoutFailureMode::Terminate
        );
    }

    #[test]
    fn test_timeout_start_failure_mode_terminate() {
        let content = "[Service]\nExecStart=/bin/true\nTimeoutStartFailureMode=terminate\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.timeout_start_failure_mode,
            super::super::TimeoutFailureMode::Terminate
        );
    }

    #[test]
    fn test_timeout_start_failure_mode_abort() {
        let content = "[Service]\nExecStart=/bin/true\nTimeoutStartFailureMode=abort\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.timeout_start_failure_mode,
            super::super::TimeoutFailureMode::Abort
        );
    }

    #[test]
    fn test_timeout_start_failure_mode_invalid() {
        let content = "[Service]\nExecStart=/bin/true\nTimeoutStartFailureMode=explode\n";
        assert!(parse_service_from_str(content).is_err());
    }

    #[test]
    fn test_timeout_start_failure_mode_case_insensitive() {
        let content = "[Service]\nExecStart=/bin/true\nTimeoutStartFailureMode=ABORT\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.timeout_start_failure_mode,
            super::super::TimeoutFailureMode::Abort
        );
    }

    // --- TimeoutStopFailureMode= ---

    #[test]
    fn test_timeout_stop_failure_mode_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.timeout_stop_failure_mode,
            super::super::TimeoutFailureMode::Terminate
        );
    }

    #[test]
    fn test_timeout_stop_failure_mode_abort() {
        let content = "[Service]\nExecStart=/bin/true\nTimeoutStopFailureMode=abort\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.timeout_stop_failure_mode,
            super::super::TimeoutFailureMode::Abort
        );
    }

    #[test]
    fn test_timeout_stop_failure_mode_invalid() {
        let content = "[Service]\nExecStart=/bin/true\nTimeoutStopFailureMode=reboot\n";
        assert!(parse_service_from_str(content).is_err());
    }

    // --- RuntimeRandomizedExtraSec= ---

    #[test]
    fn test_runtime_randomized_extra_sec_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.runtime_randomized_extra_sec.is_none());
    }

    #[test]
    fn test_runtime_randomized_extra_sec_value() {
        let content = "[Service]\nExecStart=/bin/true\nRuntimeRandomizedExtraSec=120\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.runtime_randomized_extra_sec,
            Some(Timeout::Duration(std::time::Duration::from_secs(120)))
        );
    }

    #[test]
    fn test_runtime_randomized_extra_sec_zero() {
        let content = "[Service]\nExecStart=/bin/true\nRuntimeRandomizedExtraSec=0\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.runtime_randomized_extra_sec.is_none());
    }

    #[test]
    fn test_runtime_randomized_extra_sec_empty() {
        let content = "[Service]\nExecStart=/bin/true\nRuntimeRandomizedExtraSec=\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.runtime_randomized_extra_sec.is_none());
    }

    // --- RootDirectoryStartOnly= ---

    #[test]
    fn test_root_directory_start_only_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(!config.srvc.root_directory_start_only);
    }

    #[test]
    fn test_root_directory_start_only_yes() {
        let content = "[Service]\nExecStart=/bin/true\nRootDirectoryStartOnly=yes\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.root_directory_start_only);
    }

    #[test]
    fn test_root_directory_start_only_no() {
        let content = "[Service]\nExecStart=/bin/true\nRootDirectoryStartOnly=no\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(!config.srvc.root_directory_start_only);
    }

    // --- NonBlocking= ---

    #[test]
    fn test_non_blocking_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(!config.srvc.non_blocking);
    }

    #[test]
    fn test_non_blocking_yes() {
        let content = "[Service]\nExecStart=/bin/true\nNonBlocking=yes\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.non_blocking);
    }

    #[test]
    fn test_non_blocking_true() {
        let content = "[Service]\nExecStart=/bin/true\nNonBlocking=true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.non_blocking);
    }

    #[test]
    fn test_non_blocking_no() {
        let content = "[Service]\nExecStart=/bin/true\nNonBlocking=no\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(!config.srvc.non_blocking);
    }

    // --- USBFunctionDescriptors= ---

    #[test]
    fn test_usb_function_descriptors_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.usb_function_descriptors.is_none());
    }

    #[test]
    fn test_usb_function_descriptors_value() {
        let content =
            "[Service]\nExecStart=/bin/true\nUSBFunctionDescriptors=/etc/usb/descriptors\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.usb_function_descriptors,
            Some(PathBuf::from("/etc/usb/descriptors"))
        );
    }

    #[test]
    fn test_usb_function_descriptors_empty() {
        let content = "[Service]\nExecStart=/bin/true\nUSBFunctionDescriptors=\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.usb_function_descriptors.is_none());
    }

    // --- USBFunctionStrings= ---

    #[test]
    fn test_usb_function_strings_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.usb_function_strings.is_none());
    }

    #[test]
    fn test_usb_function_strings_value() {
        let content = "[Service]\nExecStart=/bin/true\nUSBFunctionStrings=/etc/usb/strings\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.usb_function_strings,
            Some(PathBuf::from("/etc/usb/strings"))
        );
    }

    // --- OpenFile= ---

    #[test]
    fn test_open_file_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.open_file.is_empty());
    }

    #[test]
    fn test_open_file_single() {
        let content = "[Service]\nExecStart=/bin/true\nOpenFile=/var/run/secrets/key:secret\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.open_file, vec!["/var/run/secrets/key:secret"]);
    }

    #[test]
    fn test_open_file_multiple() {
        let content = "[Service]\nExecStart=/bin/true\nOpenFile=/etc/ssl/cert.pem:cert\nOpenFile=/etc/ssl/key.pem:key:read-only\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.open_file.len(), 2);
        assert_eq!(config.srvc.open_file[0], "/etc/ssl/cert.pem:cert");
        assert_eq!(config.srvc.open_file[1], "/etc/ssl/key.pem:key:read-only");
    }

    #[test]
    fn test_open_file_empty_resets() {
        let content =
            "[Service]\nExecStart=/bin/true\nOpenFile=/etc/a\nOpenFile=\nOpenFile=/etc/b\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.open_file, vec!["/etc/b"]);
    }

    // =====================================================================
    // 18 new resource-control directives
    // =====================================================================

    // --- CPUQuotaPeriodSec= ---

    #[test]
    fn test_cpu_quota_period_sec_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.cpu_quota_period_sec.is_none());
    }

    #[test]
    fn test_cpu_quota_period_sec_value() {
        let content = "[Service]\nExecStart=/bin/true\nCPUQuotaPeriodSec=10\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.cpu_quota_period_sec,
            Some(Timeout::Duration(std::time::Duration::from_secs(10)))
        );
    }

    #[test]
    fn test_cpu_quota_period_sec_empty() {
        let content = "[Service]\nExecStart=/bin/true\nCPUQuotaPeriodSec=\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.cpu_quota_period_sec.is_none());
    }

    // --- AllowedCPUs= ---

    #[test]
    fn test_allowed_cpus_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.allowed_cpus.is_none());
    }

    #[test]
    fn test_allowed_cpus_single() {
        let content = "[Service]\nExecStart=/bin/true\nAllowedCPUs=0\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.allowed_cpus, Some("0".to_owned()));
    }

    #[test]
    fn test_allowed_cpus_range() {
        let content = "[Service]\nExecStart=/bin/true\nAllowedCPUs=0-3\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.allowed_cpus, Some("0-3".to_owned()));
    }

    #[test]
    fn test_allowed_cpus_list() {
        let content = "[Service]\nExecStart=/bin/true\nAllowedCPUs=0 2 4-7\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.allowed_cpus, Some("0 2 4-7".to_owned()));
    }

    #[test]
    fn test_allowed_cpus_empty() {
        let content = "[Service]\nExecStart=/bin/true\nAllowedCPUs=\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.allowed_cpus.is_none());
    }

    // --- StartupAllowedCPUs= ---

    #[test]
    fn test_startup_allowed_cpus_value() {
        let content = "[Service]\nExecStart=/bin/true\nStartupAllowedCPUs=0-1\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.startup_allowed_cpus, Some("0-1".to_owned()));
    }

    // --- AllowedMemoryNodes= ---

    #[test]
    fn test_allowed_memory_nodes_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.allowed_memory_nodes.is_none());
    }

    #[test]
    fn test_allowed_memory_nodes_value() {
        let content = "[Service]\nExecStart=/bin/true\nAllowedMemoryNodes=0-1\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.allowed_memory_nodes, Some("0-1".to_owned()));
    }

    // --- StartupAllowedMemoryNodes= ---

    #[test]
    fn test_startup_allowed_memory_nodes_value() {
        let content = "[Service]\nExecStart=/bin/true\nStartupAllowedMemoryNodes=0\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.startup_allowed_memory_nodes,
            Some("0".to_owned())
        );
    }

    // --- DefaultMemoryMin= ---

    #[test]
    fn test_default_memory_min_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.default_memory_min.is_none());
    }

    #[test]
    fn test_default_memory_min_bytes() {
        let content = "[Service]\nExecStart=/bin/true\nDefaultMemoryMin=64M\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.default_memory_min,
            Some(super::super::MemoryLimit::Bytes(64 * 1024 * 1024))
        );
    }

    // --- DefaultMemoryLow= ---

    #[test]
    fn test_default_memory_low_bytes() {
        let content = "[Service]\nExecStart=/bin/true\nDefaultMemoryLow=128M\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.default_memory_low,
            Some(super::super::MemoryLimit::Bytes(128 * 1024 * 1024))
        );
    }

    // --- MemoryZSwapMax= ---

    #[test]
    fn test_memory_zswap_max_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.memory_zswap_max.is_none());
    }

    #[test]
    fn test_memory_zswap_max_bytes() {
        let content = "[Service]\nExecStart=/bin/true\nMemoryZSwapMax=256M\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.memory_zswap_max,
            Some(super::super::MemoryLimit::Bytes(256 * 1024 * 1024))
        );
    }

    #[test]
    fn test_memory_zswap_max_infinity() {
        let content = "[Service]\nExecStart=/bin/true\nMemoryZSwapMax=infinity\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.memory_zswap_max,
            Some(super::super::MemoryLimit::Infinity)
        );
    }

    #[test]
    fn test_memory_zswap_max_percentage() {
        let content = "[Service]\nExecStart=/bin/true\nMemoryZSwapMax=50%\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.memory_zswap_max,
            Some(super::super::MemoryLimit::Percent(50))
        );
    }

    // --- IODeviceLatencyTargetSec= ---

    #[test]
    fn test_io_device_latency_target_sec_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.io_device_latency_target_sec.is_empty());
    }

    #[test]
    fn test_io_device_latency_target_sec_value() {
        let content = "[Service]\nExecStart=/bin/true\nIODeviceLatencyTargetSec=/dev/sda 25ms\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.io_device_latency_target_sec,
            vec!["/dev/sda 25ms"]
        );
    }

    #[test]
    fn test_io_device_latency_target_sec_multiple() {
        let content = "[Service]\nExecStart=/bin/true\nIODeviceLatencyTargetSec=/dev/sda 25ms\nIODeviceLatencyTargetSec=/dev/sdb 50ms\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.io_device_latency_target_sec.len(), 2);
    }

    #[test]
    fn test_io_device_latency_target_sec_empty_resets() {
        let content = "[Service]\nExecStart=/bin/true\nIODeviceLatencyTargetSec=/dev/sda 25ms\nIODeviceLatencyTargetSec=\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.io_device_latency_target_sec.is_empty());
    }

    // --- DisableControllers= ---

    #[test]
    fn test_disable_controllers_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.disable_controllers.is_empty());
    }

    #[test]
    fn test_disable_controllers_single() {
        let content = "[Service]\nExecStart=/bin/true\nDisableControllers=memory\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.disable_controllers, vec!["memory"]);
    }

    #[test]
    fn test_disable_controllers_multiple() {
        let content = "[Service]\nExecStart=/bin/true\nDisableControllers=cpu memory io\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.disable_controllers, vec!["cpu", "memory", "io"]);
    }

    #[test]
    fn test_disable_controllers_empty_resets() {
        let content =
            "[Service]\nExecStart=/bin/true\nDisableControllers=cpu\nDisableControllers=\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.disable_controllers.is_empty());
    }

    // --- MemoryPressureThresholdSec= ---

    #[test]
    fn test_memory_pressure_threshold_sec_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.memory_pressure_threshold_sec.is_none());
    }

    #[test]
    fn test_memory_pressure_threshold_sec_value() {
        let content = "[Service]\nExecStart=/bin/true\nMemoryPressureThresholdSec=200\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.memory_pressure_threshold_sec,
            Some(Timeout::Duration(std::time::Duration::from_secs(200)))
        );
    }

    // --- IPIngressFilterPath= ---

    #[test]
    fn test_ip_ingress_filter_path_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.ip_ingress_filter_path.is_empty());
    }

    #[test]
    fn test_ip_ingress_filter_path_value() {
        let content = "[Service]\nExecStart=/bin/true\nIPIngressFilterPath=/sys/fs/bpf/ingress\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.ip_ingress_filter_path,
            vec!["/sys/fs/bpf/ingress"]
        );
    }

    #[test]
    fn test_ip_ingress_filter_path_empty_resets() {
        let content = "[Service]\nExecStart=/bin/true\nIPIngressFilterPath=/sys/fs/bpf/a\nIPIngressFilterPath=\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.ip_ingress_filter_path.is_empty());
    }

    // --- IPEgressFilterPath= ---

    #[test]
    fn test_ip_egress_filter_path_value() {
        let content = "[Service]\nExecStart=/bin/true\nIPEgressFilterPath=/sys/fs/bpf/egress\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.ip_egress_filter_path,
            vec!["/sys/fs/bpf/egress"]
        );
    }

    // --- BPFProgram= ---

    #[test]
    fn test_bpf_program_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.bpf_program.is_empty());
    }

    #[test]
    fn test_bpf_program_value() {
        let content =
            "[Service]\nExecStart=/bin/true\nBPFProgram=cgroup_skb/egress:/sys/fs/bpf/prog\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.bpf_program,
            vec!["cgroup_skb/egress:/sys/fs/bpf/prog"]
        );
    }

    #[test]
    fn test_bpf_program_empty_resets() {
        let content = "[Service]\nExecStart=/bin/true\nBPFProgram=cgroup/a\nBPFProgram=\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.bpf_program.is_empty());
    }

    // --- SocketBindAllow= ---

    #[test]
    fn test_socket_bind_allow_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.socket_bind_allow.is_empty());
    }

    #[test]
    fn test_socket_bind_allow_value() {
        let content = "[Service]\nExecStart=/bin/true\nSocketBindAllow=tcp:8080\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.socket_bind_allow, vec!["tcp:8080"]);
    }

    #[test]
    fn test_socket_bind_allow_multiple() {
        let content =
            "[Service]\nExecStart=/bin/true\nSocketBindAllow=tcp:8080\nSocketBindAllow=tcp:8443\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.socket_bind_allow.len(), 2);
    }

    // --- SocketBindDeny= ---

    #[test]
    fn test_socket_bind_deny_value() {
        let content = "[Service]\nExecStart=/bin/true\nSocketBindDeny=any\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.socket_bind_deny, vec!["any"]);
    }

    // --- RestrictNetworkInterfaces= ---

    #[test]
    fn test_restrict_network_interfaces_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.restrict_network_interfaces.is_empty());
    }

    #[test]
    fn test_restrict_network_interfaces_allow() {
        let content = "[Service]\nExecStart=/bin/true\nRestrictNetworkInterfaces=eth0 wlan0\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.restrict_network_interfaces,
            vec!["eth0", "wlan0"]
        );
    }

    #[test]
    fn test_restrict_network_interfaces_deny() {
        let content = "[Service]\nExecStart=/bin/true\nRestrictNetworkInterfaces=~docker0 veth+\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.restrict_network_interfaces,
            vec!["~docker0", "veth+"]
        );
    }

    #[test]
    fn test_restrict_network_interfaces_empty_resets() {
        let content = "[Service]\nExecStart=/bin/true\nRestrictNetworkInterfaces=eth0\nRestrictNetworkInterfaces=\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.restrict_network_interfaces.is_empty());
    }

    // --- NFTSet= ---

    #[test]
    fn test_nft_set_default() {
        let content = "[Service]\nExecStart=/bin/true\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.nft_set.is_empty());
    }

    #[test]
    fn test_nft_set_value() {
        let content = "[Service]\nExecStart=/bin/true\nNFTSet=inet:filter:cgroup_set\n";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(config.srvc.nft_set, vec!["inet:filter:cgroup_set"]);
    }

    #[test]
    fn test_nft_set_empty_resets() {
        let content = "[Service]\nExecStart=/bin/true\nNFTSet=inet:a:b\nNFTSet=\n";
        let config = parse_service_from_str(content).unwrap();
        assert!(config.srvc.nft_set.is_empty());
    }

    // =====================================================================
    // Combined / integration tests for new directives
    // =====================================================================

    #[test]
    fn test_all_new_service_directives_together() {
        let content = "\
[Service]
ExecStart=/bin/daemon
GuessMainPID=no
TimeoutStartFailureMode=abort
TimeoutStopFailureMode=abort
RuntimeRandomizedExtraSec=60
RootDirectoryStartOnly=yes
NonBlocking=yes
USBFunctionDescriptors=/etc/usb/desc
USBFunctionStrings=/etc/usb/str
OpenFile=/var/secrets/key:keyfile:read-only
";
        let config = parse_service_from_str(content).unwrap();
        assert!(!config.srvc.guess_main_pid);
        assert_eq!(
            config.srvc.timeout_start_failure_mode,
            super::super::TimeoutFailureMode::Abort
        );
        assert_eq!(
            config.srvc.timeout_stop_failure_mode,
            super::super::TimeoutFailureMode::Abort
        );
        assert_eq!(
            config.srvc.runtime_randomized_extra_sec,
            Some(Timeout::Duration(std::time::Duration::from_secs(60)))
        );
        assert!(config.srvc.root_directory_start_only);
        assert!(config.srvc.non_blocking);
        assert_eq!(
            config.srvc.usb_function_descriptors,
            Some(PathBuf::from("/etc/usb/desc"))
        );
        assert_eq!(
            config.srvc.usb_function_strings,
            Some(PathBuf::from("/etc/usb/str"))
        );
        assert_eq!(
            config.srvc.open_file,
            vec!["/var/secrets/key:keyfile:read-only"]
        );
    }

    #[test]
    fn test_all_new_resource_control_directives_together() {
        let content = "\
[Service]
ExecStart=/bin/daemon
CPUQuotaPeriodSec=5
AllowedCPUs=0-3
StartupAllowedCPUs=0-1
AllowedMemoryNodes=0
StartupAllowedMemoryNodes=0
DefaultMemoryMin=32M
DefaultMemoryLow=64M
MemoryZSwapMax=512M
IODeviceLatencyTargetSec=/dev/sda 10ms
DisableControllers=cpu io
MemoryPressureThresholdSec=100
IPIngressFilterPath=/sys/fs/bpf/in
IPEgressFilterPath=/sys/fs/bpf/out
BPFProgram=cgroup_skb/egress:/sys/fs/bpf/prog
SocketBindAllow=tcp:443
SocketBindDeny=any
RestrictNetworkInterfaces=eth0
NFTSet=inet:filter:service_set
";
        let config = parse_service_from_str(content).unwrap();
        assert_eq!(
            config.srvc.cpu_quota_period_sec,
            Some(Timeout::Duration(std::time::Duration::from_secs(5)))
        );
        assert_eq!(config.srvc.allowed_cpus, Some("0-3".to_owned()));
        assert_eq!(config.srvc.startup_allowed_cpus, Some("0-1".to_owned()));
        assert_eq!(config.srvc.allowed_memory_nodes, Some("0".to_owned()));
        assert_eq!(
            config.srvc.startup_allowed_memory_nodes,
            Some("0".to_owned())
        );
        assert_eq!(
            config.srvc.default_memory_min,
            Some(super::super::MemoryLimit::Bytes(32 * 1024 * 1024))
        );
        assert_eq!(
            config.srvc.default_memory_low,
            Some(super::super::MemoryLimit::Bytes(64 * 1024 * 1024))
        );
        assert_eq!(
            config.srvc.memory_zswap_max,
            Some(super::super::MemoryLimit::Bytes(512 * 1024 * 1024))
        );
        assert_eq!(
            config.srvc.io_device_latency_target_sec,
            vec!["/dev/sda 10ms"]
        );
        assert_eq!(config.srvc.disable_controllers, vec!["cpu", "io"]);
        assert_eq!(
            config.srvc.memory_pressure_threshold_sec,
            Some(Timeout::Duration(std::time::Duration::from_secs(100)))
        );
        assert_eq!(config.srvc.ip_ingress_filter_path, vec!["/sys/fs/bpf/in"]);
        assert_eq!(config.srvc.ip_egress_filter_path, vec!["/sys/fs/bpf/out"]);
        assert_eq!(
            config.srvc.bpf_program,
            vec!["cgroup_skb/egress:/sys/fs/bpf/prog"]
        );
        assert_eq!(config.srvc.socket_bind_allow, vec!["tcp:443"]);
        assert_eq!(config.srvc.socket_bind_deny, vec!["any"]);
        assert_eq!(config.srvc.restrict_network_interfaces, vec!["eth0"]);
        assert_eq!(config.srvc.nft_set, vec!["inet:filter:service_set"]);
    }

    #[test]
    fn test_new_directives_defaults_in_no_service_section() {
        let content = "[Unit]\nDescription=Test\n";
        let config = parse_service_from_str(content).unwrap();
        // Service directives
        assert!(config.srvc.guess_main_pid);
        assert_eq!(
            config.srvc.timeout_start_failure_mode,
            super::super::TimeoutFailureMode::Terminate
        );
        assert_eq!(
            config.srvc.timeout_stop_failure_mode,
            super::super::TimeoutFailureMode::Terminate
        );
        assert!(config.srvc.runtime_randomized_extra_sec.is_none());
        assert!(!config.srvc.root_directory_start_only);
        assert!(!config.srvc.non_blocking);
        assert!(config.srvc.usb_function_descriptors.is_none());
        assert!(config.srvc.usb_function_strings.is_none());
        assert!(config.srvc.open_file.is_empty());
        // Resource-control directives
        assert!(config.srvc.cpu_quota_period_sec.is_none());
        assert!(config.srvc.allowed_cpus.is_none());
        assert!(config.srvc.startup_allowed_cpus.is_none());
        assert!(config.srvc.allowed_memory_nodes.is_none());
        assert!(config.srvc.startup_allowed_memory_nodes.is_none());
        assert!(config.srvc.default_memory_min.is_none());
        assert!(config.srvc.default_memory_low.is_none());
        assert!(config.srvc.memory_zswap_max.is_none());
        assert!(config.srvc.io_device_latency_target_sec.is_empty());
        assert!(config.srvc.disable_controllers.is_empty());
        assert!(config.srvc.memory_pressure_threshold_sec.is_none());
        assert!(config.srvc.ip_ingress_filter_path.is_empty());
        assert!(config.srvc.ip_egress_filter_path.is_empty());
        assert!(config.srvc.bpf_program.is_empty());
        assert!(config.srvc.socket_bind_allow.is_empty());
        assert!(config.srvc.socket_bind_deny.is_empty());
        assert!(config.srvc.restrict_network_interfaces.is_empty());
        assert!(config.srvc.nft_set.is_empty());
    }

    #[test]
    fn test_new_directives_coexist_with_existing() {
        let content = "\
[Service]
Type=notify
ExecStart=/bin/daemon
Restart=always
WatchdogSec=30
GuessMainPID=no
TimeoutStartFailureMode=abort
AllowedCPUs=0-7
MemoryZSwapMax=1G
CPUQuota=200%
MemoryMax=2G
";
        let config = parse_service_from_str(content).unwrap();
        // Existing directives still work
        assert_eq!(config.srvc.srcv_type, ServiceType::Notify);
        assert_eq!(config.srvc.restart, ServiceRestart::Always);
        assert!(config.srvc.watchdog_sec.is_some());
        assert_eq!(config.srvc.cpu_quota, Some(200));
        assert!(config.srvc.memory_max.is_some());
        // New directives work alongside
        assert!(!config.srvc.guess_main_pid);
        assert_eq!(
            config.srvc.timeout_start_failure_mode,
            super::super::TimeoutFailureMode::Abort
        );
        assert_eq!(config.srvc.allowed_cpus, Some("0-7".to_owned()));
        assert_eq!(
            config.srvc.memory_zswap_max,
            Some(super::super::MemoryLimit::Bytes(1024 * 1024 * 1024))
        );
    }

    // ── Command line prefix tests ─────────────────────────────────────

    #[test]
    fn test_plus_prefix_execstart() {
        let config = parse_service_from_str(
            r#"
            [Service]
            ExecStart = +/usr/bin/foo arg1 arg2
            "#,
        )
        .unwrap();
        let exec = config.srvc.exec.last().unwrap();
        assert_eq!(exec.cmd, "/usr/bin/foo");
        assert_eq!(exec.args, vec!["arg1", "arg2"]);
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::Plus)
        );
    }

    #[test]
    fn test_colon_prefix_execstart() {
        let config = parse_service_from_str(
            r#"
            [Service]
            ExecStart = :/usr/bin/foo arg1
            "#,
        )
        .unwrap();
        let exec = config.srvc.exec.last().unwrap();
        assert_eq!(exec.cmd, "/usr/bin/foo");
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::Colon)
        );
    }

    #[test]
    fn test_exclamation_prefix_execstart() {
        let config = parse_service_from_str(
            r#"
            [Service]
            ExecStart = !/usr/bin/foo arg1
            "#,
        )
        .unwrap();
        let exec = config.srvc.exec.last().unwrap();
        assert_eq!(exec.cmd, "/usr/bin/foo");
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::Exclamation)
        );
    }

    #[test]
    fn test_double_exclamation_prefix_execstart() {
        let config = parse_service_from_str(
            r#"
            [Service]
            ExecStart = !!/usr/bin/foo arg1
            "#,
        )
        .unwrap();
        let exec = config.srvc.exec.last().unwrap();
        assert_eq!(exec.cmd, "/usr/bin/foo");
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::DoubleExclamation)
        );
    }

    #[test]
    fn test_plus_prefix_combined_with_minus() {
        let config = parse_service_from_str(
            r#"
            [Service]
            ExecStart = -+/usr/bin/foo arg1
            "#,
        )
        .unwrap();
        let exec = config.srvc.exec.last().unwrap();
        assert_eq!(exec.cmd, "/usr/bin/foo");
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::Minus)
        );
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::Plus)
        );
    }

    #[test]
    fn test_plus_prefix_combined_with_at() {
        let config = parse_service_from_str(
            r#"
            [Service]
            ExecStart = +@/usr/bin/foo bar arg1
            "#,
        )
        .unwrap();
        let exec = config.srvc.exec.last().unwrap();
        assert_eq!(exec.cmd, "/usr/bin/foo");
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::Plus)
        );
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::AtSign)
        );
    }

    #[test]
    fn test_colon_prefix_combined_with_minus() {
        let config = parse_service_from_str(
            r#"
            [Service]
            ExecStart = -:/usr/bin/foo
            "#,
        )
        .unwrap();
        let exec = config.srvc.exec.last().unwrap();
        assert_eq!(exec.cmd, "/usr/bin/foo");
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::Minus)
        );
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::Colon)
        );
    }

    #[test]
    fn test_exclamation_prefix_combined_with_minus_and_at() {
        let config = parse_service_from_str(
            r#"
            [Service]
            ExecStart = -!@/usr/bin/foo bar arg1
            "#,
        )
        .unwrap();
        let exec = config.srvc.exec.last().unwrap();
        assert_eq!(exec.cmd, "/usr/bin/foo");
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::Minus)
        );
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::Exclamation)
        );
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::AtSign)
        );
    }

    #[test]
    fn test_plus_prefix_execstartpre() {
        let config = parse_service_from_str(
            r#"
            [Service]
            ExecStartPre = +/usr/bin/setup --init
            ExecStart = /usr/bin/main
            "#,
        )
        .unwrap();
        let pre = &config.srvc.startpre;
        assert_eq!(pre.len(), 1);
        assert_eq!(pre[0].cmd, "/usr/bin/setup");
        assert!(
            pre[0]
                .prefixes
                .contains(&super::super::CommandlinePrefix::Plus)
        );
    }

    #[test]
    fn test_plus_prefix_execstop() {
        let config = parse_service_from_str(
            r#"
            [Service]
            ExecStart = /usr/bin/main
            ExecStop = +/usr/bin/cleanup --force
            "#,
        )
        .unwrap();
        let stop = &config.srvc.stop;
        assert_eq!(stop.len(), 1);
        assert_eq!(stop[0].cmd, "/usr/bin/cleanup");
        assert!(
            stop[0]
                .prefixes
                .contains(&super::super::CommandlinePrefix::Plus)
        );
    }

    #[test]
    fn test_plus_prefix_execstoppost() {
        let config = parse_service_from_str(
            r#"
            [Service]
            ExecStart = /usr/bin/main
            ExecStopPost = +/usr/bin/teardown
            "#,
        )
        .unwrap();
        let stop_post = &config.srvc.stoppost;
        assert_eq!(stop_post.len(), 1);
        assert_eq!(stop_post[0].cmd, "/usr/bin/teardown");
        assert!(
            stop_post[0]
                .prefixes
                .contains(&super::super::CommandlinePrefix::Plus)
        );
    }

    #[test]
    fn test_all_prefixes_combined() {
        // Test that minus, plus, colon can coexist (even if unusual).
        // systemd allows any combination of prefixes on a single command.
        let config = parse_service_from_str(
            r#"
            [Service]
            ExecStart = -+:/usr/bin/foo
            "#,
        )
        .unwrap();
        let exec = config.srvc.exec.last().unwrap();
        assert_eq!(exec.cmd, "/usr/bin/foo");
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::Minus)
        );
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::Plus)
        );
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::Colon)
        );
    }

    #[test]
    fn test_double_exclamation_not_confused_with_single() {
        // Ensure "!!" is parsed as DoubleExclamation, not two Exclamation prefixes
        let config = parse_service_from_str(
            r#"
            [Service]
            ExecStart = !!/usr/bin/foo
            "#,
        )
        .unwrap();
        let exec = config.srvc.exec.last().unwrap();
        assert_eq!(exec.prefixes.len(), 1);
        assert!(
            exec.prefixes
                .contains(&super::super::CommandlinePrefix::DoubleExclamation)
        );
        assert!(
            !exec
                .prefixes
                .contains(&super::super::CommandlinePrefix::Exclamation)
        );
    }

    #[test]
    fn test_prefix_count_preserved() {
        let config = parse_service_from_str(
            r#"
            [Service]
            ExecStart = -+@/usr/bin/foo bar
            "#,
        )
        .unwrap();
        let exec = config.srvc.exec.last().unwrap();
        assert_eq!(exec.prefixes.len(), 3);
        assert_eq!(exec.prefixes[0], super::super::CommandlinePrefix::Minus);
        assert_eq!(exec.prefixes[1], super::super::CommandlinePrefix::Plus);
        assert_eq!(exec.prefixes[2], super::super::CommandlinePrefix::AtSign);
    }

    #[test]
    fn test_parse_oneshot_with_multiple_exec_and_startpost() {
        // This mirrors testservice-failure-exit-handler-68.service from TEST-68
        let content = r#"[Service]
# repeat the check to make sure that values are set correctly on repeated invocations
Type=oneshot
ExecStartPre=/tmp/check_on_failure.sh
ExecStartPre=/tmp/check_on_failure.sh
ExecStart=/tmp/check_on_failure.sh
ExecStart=/tmp/check_on_failure.sh
ExecStartPost=test -z ''
"#;
        let parsed = super::super::parse_file(content).unwrap();
        let result = parse_service(
            parsed,
            &std::path::PathBuf::from("/test/failure-handler.service"),
        );
        assert!(
            result.is_ok(),
            "Failed to parse oneshot service with multiple ExecStart and ExecStartPost: {:?}",
            result.err()
        );
        let config = result.unwrap();
        // exec is Vec<Commandline> — all ExecStart= lines are stored
        assert_eq!(config.srvc.exec.len(), 2);
        assert_eq!(config.srvc.startpre.len(), 2);
        assert_eq!(config.srvc.startpost.len(), 1);
    }
}

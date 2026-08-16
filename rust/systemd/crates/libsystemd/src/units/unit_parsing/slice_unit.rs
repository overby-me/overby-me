use log::trace;

use crate::units::{
    ParsedCommonConfig, ParsedFile, ParsedSliceConfig, ParsedSliceSection, ParsingErrorReason,
    parse_install_section, parse_unit_section,
};
use std::path::PathBuf;

pub fn parse_slice(
    parsed_file: ParsedFile,
    path: &PathBuf,
) -> Result<ParsedSliceConfig, ParsingErrorReason> {
    let mut install_config = None;
    let mut unit_config = None;
    let mut slice_section = ParsedSliceSection::default();

    for (name, section) in parsed_file {
        match name.as_str() {
            "[Unit]" => {
                unit_config = Some(parse_unit_section(section)?);
            }
            "[Install]" => {
                install_config = Some(parse_install_section(section)?);
            }
            "[Slice]" => {
                parse_slice_section(&section, &mut slice_section)?;
            }
            _ if name.starts_with("[X-") || name.starts_with("[x-") => {
                trace!("Silently ignoring vendor extension section in slice unit {path:?}: {name}");
            }
            _ => {
                trace!("Ignoring unknown section in slice unit {path:?}: {name}");
            }
        }
    }

    Ok(ParsedSliceConfig {
        common: ParsedCommonConfig {
            name: path.file_name().unwrap().to_str().unwrap().to_owned(),
            unit: unit_config.unwrap_or_default(),
            install: install_config.unwrap_or_default(),
            fragment_path: Some(path.clone()),
        },
        slice: slice_section,
    })
}

/// Parse a `ConcurrencyHardMax=`/`ConcurrencySoftMax=` value: empty or
/// "infinity" means no limit (None); otherwise a plain unsigned count.
fn parse_concurrency_max(value: &str, key: &str) -> Result<Option<u32>, ParsingErrorReason> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("infinity") {
        return Ok(None);
    }
    trimmed
        .parse::<u32>()
        .map(Some)
        .map_err(|_| ParsingErrorReason::Generic(format!("{key} is not a valid count: {value}")))
}

fn parse_slice_section(
    section: &std::collections::HashMap<String, Vec<(u32, String)>>,
    slice: &mut ParsedSliceSection,
) -> Result<(), ParsingErrorReason> {
    // Collect all key-value pairs sorted by line number so that
    // later assignments override earlier ones (matching systemd semantics).
    let mut entries: Vec<(u32, &str, &str)> = Vec::new();
    for (key, values) in section {
        for (line, value) in values {
            entries.push((*line, key.as_str(), value.as_str()));
        }
    }
    entries.sort_by_key(|(line, _, _)| *line);

    for (_line, key, value) in entries {
        // Note: the unit file parser (`parse_section`) converts all keys to
        // uppercase, so we match against uppercase names here.
        match key {
            // --- Memory controller ---
            "MEMORYMIN" => {
                slice.memory_min = parse_memory_limit_opt(value, "MemoryMin")?;
            }
            "MEMORYLOW" => {
                slice.memory_low = parse_memory_limit_opt(value, "MemoryLow")?;
            }
            "MEMORYHIGH" => {
                slice.memory_high = parse_memory_limit_opt(value, "MemoryHigh")?;
            }
            "MEMORYMAX" => {
                slice.memory_max = parse_memory_limit_opt(value, "MemoryMax")?;
            }
            "MEMORYSWAPMAX" => {
                slice.memory_swap_max = parse_memory_limit_opt(value, "MemorySwapMax")?;
            }

            // --- CPU controller ---
            "CPUWEIGHT" => {
                slice.cpu_weight = super::parse_cpu_weight(value)
                    .map_err(|e| ParsingErrorReason::Generic(format!("CPUWeight: {e}")))?;
            }
            "STARTUPCPUWEIGHT" => {
                slice.startup_cpu_weight = super::parse_cpu_weight(value)
                    .map_err(|e| ParsingErrorReason::Generic(format!("StartupCPUWeight: {e}")))?;
            }
            "CPUQUOTA" => {
                slice.cpu_quota = super::parse_cpu_quota(value)
                    .map_err(|e| ParsingErrorReason::Generic(format!("CPUQuota: {e}")))?;
            }

            // --- IO controller ---
            "IOWEIGHT" => {
                slice.io_weight = super::parse_io_weight(value)
                    .map_err(|e| ParsingErrorReason::Generic(format!("IOWeight: {e}")))?;
            }
            "STARTUPIOWEIGHT" => {
                slice.startup_io_weight = super::parse_io_weight(value)
                    .map_err(|e| ParsingErrorReason::Generic(format!("StartupIOWeight: {e}")))?;
            }
            "IODEVICEWEIGHT" => {
                parse_io_device_list(value, &mut slice.io_device_weight, "IODeviceWeight")?;
            }
            "IOREADBANDWIDTHMAX" => {
                parse_io_device_list(
                    value,
                    &mut slice.io_read_bandwidth_max,
                    "IOReadBandwidthMax",
                )?;
            }
            "IOWRITEBANDWIDTHMAX" => {
                parse_io_device_list(
                    value,
                    &mut slice.io_write_bandwidth_max,
                    "IOWriteBandwidthMax",
                )?;
            }
            "IOREADIOPSMAX" => {
                parse_io_device_list(value, &mut slice.io_read_iops_max, "IOReadIOPSMax")?;
            }
            "IOWRITEIOPSMAX" => {
                parse_io_device_list(value, &mut slice.io_write_iops_max, "IOWriteIOPSMax")?;
            }

            // --- Tasks (pids) controller ---
            "TASKSMAX" => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    slice.tasks_max = None;
                } else if trimmed.eq_ignore_ascii_case("infinity") {
                    slice.tasks_max = Some(super::TasksMax::Infinity);
                } else if let Some(pct) = trimmed.strip_suffix('%') {
                    // Accept both integer ("50%") and scaled ("40.00%")
                    // percentages; set-property persists TasksMaxScale as the
                    // latter (e.g. "TasksMax=40.00%").
                    let pct_val = pct.trim().parse::<f64>().map_err(|_| {
                        ParsingErrorReason::Generic(format!(
                            "TasksMax percentage is not a valid number: {value}"
                        ))
                    })?;
                    slice.tasks_max = Some(super::TasksMax::Percent(pct_val.round() as u64));
                } else {
                    let num = trimmed.parse::<u64>().map_err(|_| {
                        ParsingErrorReason::Generic(format!(
                            "TasksMax is not a valid value: {value}"
                        ))
                    })?;
                    slice.tasks_max = Some(super::TasksMax::Value(num));
                }
            }

            // --- Concurrency limits ---
            "CONCURRENCYSOFTMAX" => {
                slice.concurrency_soft_max = parse_concurrency_max(value, "ConcurrencySoftMax")?;
            }
            "CONCURRENCYHARDMAX" => {
                slice.concurrency_hard_max = parse_concurrency_max(value, "ConcurrencyHardMax")?;
            }

            // --- Delegation ---
            "DELEGATE" => {
                let trimmed = value.trim();
                if string_to_bool(trimmed) {
                    slice.delegate = super::Delegate::Yes;
                } else if trimmed.eq_ignore_ascii_case("no")
                    || trimmed.eq_ignore_ascii_case("false")
                    || trimmed == "0"
                    || trimmed.is_empty()
                {
                    slice.delegate = super::Delegate::No;
                } else {
                    // Treat as a space-separated list of controller names
                    let controllers: Vec<String> =
                        trimmed.split_whitespace().map(|s| s.to_owned()).collect();
                    if controllers.is_empty() {
                        slice.delegate = super::Delegate::No;
                    } else {
                        slice.delegate = super::Delegate::Controllers(controllers);
                    }
                }
            }

            // --- Accounting toggles ---
            "CPUACCOUNTING" => {
                slice.cpu_accounting = Some(string_to_bool(value.trim()));
            }
            "MEMORYACCOUNTING" => {
                slice.memory_accounting = Some(string_to_bool(value.trim()));
            }
            "IOACCOUNTING" => {
                slice.io_accounting = Some(string_to_bool(value.trim()));
            }
            "TASKSACCOUNTING" => {
                slice.tasks_accounting = Some(string_to_bool(value.trim()));
            }

            // --- Device policy ---
            "DEVICEALLOW" => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    slice.device_allow.clear();
                } else {
                    slice.device_allow.push(trimmed.to_owned());
                }
            }
            "DEVICEPOLICY" => {
                slice.device_policy = match value.trim().to_lowercase().as_str() {
                    "auto" | "" => super::DevicePolicy::Auto,
                    "closed" => super::DevicePolicy::Closed,
                    "strict" => super::DevicePolicy::Strict,
                    other => {
                        return Err(ParsingErrorReason::UnknownSetting(
                            "DevicePolicy".to_owned(),
                            other.to_owned(),
                        ));
                    }
                };
            }

            // --- IP address filtering ---
            "IPADDRESSALLOW" => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    slice.ip_address_allow.clear();
                } else {
                    for addr in trimmed.split_whitespace() {
                        slice.ip_address_allow.push(addr.to_owned());
                    }
                }
            }
            "IPADDRESSDENY" => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    slice.ip_address_deny.clear();
                } else {
                    for addr in trimmed.split_whitespace() {
                        slice.ip_address_deny.push(addr.to_owned());
                    }
                }
            }

            // --- Managed OOM ---
            "MANAGEDOOMSWAP" => {
                let trimmed = value.trim();
                slice.managed_oom_swap = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                };
            }
            "MANAGEDOOMMEMORYPRESSURE" => {
                let trimmed = value.trim();
                slice.managed_oom_memory_pressure = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                };
            }
            "MANAGEDOOMMEMORYPRESSURELIMIT" => {
                let trimmed = value.trim();
                slice.managed_oom_memory_pressure_limit = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                };
            }
            "MANAGEDOOMPREFERENCE" => {
                let trimmed = value.trim();
                slice.managed_oom_preference = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                };
            }

            // --- Memory pressure watch ---
            "MEMORYPRESSUREWATCH" => {
                slice.memory_pressure_watch = match value.trim().to_lowercase().as_str() {
                    "auto" | "" => super::MemoryPressureWatch::Auto,
                    "on" | "yes" | "true" | "1" => super::MemoryPressureWatch::On,
                    "off" | "no" | "false" | "0" => super::MemoryPressureWatch::Off,
                    "skip" => super::MemoryPressureWatch::Skip,
                    other => {
                        return Err(ParsingErrorReason::UnknownSetting(
                            "MemoryPressureWatch".to_owned(),
                            other.to_owned(),
                        ));
                    }
                };
            }

            // --- New resource-control directives ---
            "CPUQUOTAPERIODSEC" => {
                let trimmed = value.trim();
                slice.cpu_quota_period_sec = if trimmed.is_empty() {
                    None
                } else {
                    Some(super::service_unit::parse_timeout(trimmed))
                };
            }
            "ALLOWEDCPUS" => {
                let trimmed = value.trim();
                slice.allowed_cpus = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                };
            }
            "STARTUPALLOWEDCPUS" => {
                let trimmed = value.trim();
                slice.startup_allowed_cpus = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                };
            }
            "ALLOWEDMEMORYNODES" => {
                let trimmed = value.trim();
                slice.allowed_memory_nodes = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                };
            }
            "STARTUPALLOWEDMEMORYNODES" => {
                let trimmed = value.trim();
                slice.startup_allowed_memory_nodes = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                };
            }
            "DEFAULTMEMORYMIN" => {
                slice.default_memory_min = parse_memory_limit_opt(value, "DefaultMemoryMin")?;
            }
            "DEFAULTMEMORYLOW" => {
                slice.default_memory_low = parse_memory_limit_opt(value, "DefaultMemoryLow")?;
            }
            "MEMORYZSWAPMAX" => {
                slice.memory_zswap_max = parse_memory_limit_opt(value, "MemoryZSwapMax")?;
            }
            "IODEVICELATENCYTARGETSEC" => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    slice.io_device_latency_target_sec.clear();
                } else {
                    slice.io_device_latency_target_sec.push(trimmed.to_owned());
                }
            }
            "DISABLECONTROLLERS" => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    slice.disable_controllers.clear();
                } else {
                    for token in trimmed.split_whitespace() {
                        slice.disable_controllers.push(token.to_owned());
                    }
                }
            }
            "MEMORYPRESSURETHRESHOLDSEC" => {
                let trimmed = value.trim();
                slice.memory_pressure_threshold_sec = if trimmed.is_empty() {
                    None
                } else {
                    Some(super::service_unit::parse_timeout(trimmed))
                };
            }
            "IPINGRESSFILTERPATH" => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    slice.ip_ingress_filter_path.clear();
                } else {
                    slice.ip_ingress_filter_path.push(trimmed.to_owned());
                }
            }
            "IPEGRESSFILTERPATH" => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    slice.ip_egress_filter_path.clear();
                } else {
                    slice.ip_egress_filter_path.push(trimmed.to_owned());
                }
            }
            "BPFPROGRAM" => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    slice.bpf_program.clear();
                } else {
                    slice.bpf_program.push(trimmed.to_owned());
                }
            }
            "SOCKETBINDALLOW" => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    slice.socket_bind_allow.clear();
                } else {
                    slice.socket_bind_allow.push(trimmed.to_owned());
                }
            }
            "SOCKETBINDDENY" => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    slice.socket_bind_deny.clear();
                } else {
                    slice.socket_bind_deny.push(trimmed.to_owned());
                }
            }
            "RESTRICTNETWORKINTERFACES" => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    slice.restrict_network_interfaces.clear();
                } else {
                    for token in trimmed.split_whitespace() {
                        slice.restrict_network_interfaces.push(token.to_owned());
                    }
                }
            }
            "NFTSET" => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    slice.nft_set.clear();
                } else {
                    slice.nft_set.push(trimmed.to_owned());
                }
            }
            "DELEGATESUBGROUP" => {
                let trimmed = value.trim();
                slice.delegate_subgroup = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                };
            }

            other => {
                if other.starts_with("X-") || other.starts_with("x-") {
                    trace!(
                        "Silently ignoring vendor extension in [Slice] section: {other}={value}"
                    );
                } else {
                    trace!("Ignoring unsupported setting in [Slice] section: {other}={value}");
                }
            }
        }
    }

    Ok(())
}

/// Parse a memory limit value (bytes with suffix, percentage, or "infinity")
/// returning `None` for empty strings.
fn parse_memory_limit_opt(
    value: &str,
    name: &str,
) -> Result<Option<super::MemoryLimit>, ParsingErrorReason> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    super::parse_memory_limit(trimmed)
        .map_err(|e| ParsingErrorReason::Generic(format!("{name}: {e}")))
}

/// Parse and accumulate an I/O device limit entry. An empty value resets the
/// list (matching systemd semantics).
fn parse_io_device_list(
    value: &str,
    entries: &mut Vec<super::IoDeviceLimit>,
    name: &str,
) -> Result<(), ParsingErrorReason> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        entries.clear();
        return Ok(());
    }
    if let Some(limit) = super::parse_io_device_limit(trimmed)
        .map_err(|e| ParsingErrorReason::Generic(format!("{name}: {e}")))?
    {
        entries.push(limit);
    }
    Ok(())
}

fn string_to_bool(val: &str) -> bool {
    matches!(
        val.to_lowercase().as_str(),
        "true" | "yes" | "on" | "1" | "y"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::unit_parsing::unit_parser::parse_file;

    fn parse_slice_from_str(content: &str) -> Result<ParsedSliceConfig, ParsingErrorReason> {
        let path = PathBuf::from("/test/test.slice");
        let parsed_file = parse_file(content)?;
        parse_slice(parsed_file, &path)
    }

    #[test]
    fn test_empty_slice() {
        let content = "[Unit]\nDescription=Test Slice\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.common.name, "test.slice");
        assert!(config.slice.memory_max.is_none());
        assert!(config.slice.cpu_quota.is_none());
        assert!(config.slice.tasks_max.is_none());
    }

    #[test]
    fn test_memory_max_bytes() {
        let content = "[Slice]\nMemoryMax=1073741824\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.memory_max.is_some());
        if let Some(super::super::MemoryLimit::Bytes(b)) = config.slice.memory_max {
            assert_eq!(b, 1073741824);
        } else {
            panic!("Expected MemoryLimit::Bytes");
        }
    }

    #[test]
    fn test_memory_max_suffix() {
        let content = "[Slice]\nMemoryMax=1G\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.memory_max.is_some());
        if let Some(super::super::MemoryLimit::Bytes(b)) = config.slice.memory_max {
            assert_eq!(b, 1024 * 1024 * 1024);
        } else {
            panic!("Expected MemoryLimit::Bytes");
        }
    }

    #[test]
    fn test_memory_max_percentage() {
        let content = "[Slice]\nMemoryMax=80%\n";
        let config = parse_slice_from_str(content).unwrap();
        if let Some(super::super::MemoryLimit::Percent(p)) = config.slice.memory_max {
            assert_eq!(p, 80);
        } else {
            panic!("Expected MemoryLimit::Percent");
        }
    }

    #[test]
    fn test_memory_max_infinity() {
        let content = "[Slice]\nMemoryMax=infinity\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.memory_max,
            Some(super::super::MemoryLimit::Infinity)
        );
    }

    #[test]
    fn test_memory_min_low_high() {
        let content = "[Slice]\nMemoryMin=64M\nMemoryLow=128M\nMemoryHigh=2G\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.memory_min.is_some());
        assert!(config.slice.memory_low.is_some());
        assert!(config.slice.memory_high.is_some());
    }

    #[test]
    fn test_memory_swap_max() {
        let content = "[Slice]\nMemorySwapMax=512M\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.memory_swap_max.is_some());
    }

    #[test]
    fn test_cpu_weight() {
        let content = "[Slice]\nCPUWeight=200\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.cpu_weight, Some(200));
    }

    #[test]
    fn test_cpu_weight_idle() {
        let content = "[Slice]\nCPUWeight=idle\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.cpu_weight, Some(1));
    }

    #[test]
    fn test_startup_cpu_weight() {
        let content = "[Slice]\nStartupCPUWeight=500\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.startup_cpu_weight, Some(500));
    }

    #[test]
    fn test_cpu_quota() {
        let content = "[Slice]\nCPUQuota=20%\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.cpu_quota, Some(20));
    }

    #[test]
    fn test_cpu_quota_200() {
        let content = "[Slice]\nCPUQuota=200%\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.cpu_quota, Some(200));
    }

    #[test]
    fn test_io_weight() {
        let content = "[Slice]\nIOWeight=500\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.io_weight, Some(500));
    }

    #[test]
    fn test_startup_io_weight() {
        let content = "[Slice]\nStartupIOWeight=100\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.startup_io_weight, Some(100));
    }

    #[test]
    fn test_tasks_max_value() {
        let content = "[Slice]\nTasksMax=4096\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.tasks_max,
            Some(super::super::TasksMax::Value(4096))
        );
    }

    #[test]
    fn test_tasks_max_infinity() {
        let content = "[Slice]\nTasksMax=infinity\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.tasks_max,
            Some(super::super::TasksMax::Infinity)
        );
    }

    #[test]
    fn test_tasks_max_percent() {
        let content = "[Slice]\nTasksMax=50%\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.tasks_max,
            Some(super::super::TasksMax::Percent(50))
        );
    }

    #[test]
    fn test_delegate_yes() {
        let content = "[Slice]\nDelegate=yes\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.delegate, super::super::Delegate::Yes);
    }

    #[test]
    fn test_delegate_no() {
        let content = "[Slice]\nDelegate=no\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.delegate, super::super::Delegate::No);
    }

    #[test]
    fn test_delegate_controllers() {
        let content = "[Slice]\nDelegate=cpu memory io\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.delegate,
            super::super::Delegate::Controllers(vec![
                "cpu".to_owned(),
                "memory".to_owned(),
                "io".to_owned()
            ])
        );
    }

    #[test]
    fn test_cpu_accounting() {
        let content = "[Slice]\nCPUAccounting=yes\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.cpu_accounting, Some(true));
    }

    #[test]
    fn test_memory_accounting_false() {
        let content = "[Slice]\nMemoryAccounting=no\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.memory_accounting, Some(false));
    }

    #[test]
    fn test_io_accounting() {
        let content = "[Slice]\nIOAccounting=true\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.io_accounting, Some(true));
    }

    #[test]
    fn test_tasks_accounting() {
        let content = "[Slice]\nTasksAccounting=yes\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.tasks_accounting, Some(true));
    }

    #[test]
    fn test_device_allow() {
        let content = "[Slice]\nDeviceAllow=/dev/null rwm\nDeviceAllow=/dev/zero r\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.device_allow.len(), 2);
        assert_eq!(config.slice.device_allow[0], "/dev/null rwm");
        assert_eq!(config.slice.device_allow[1], "/dev/zero r");
    }

    #[test]
    fn test_device_allow_reset() {
        let content = "[Slice]\nDeviceAllow=/dev/null rwm\nDeviceAllow=\nDeviceAllow=/dev/zero r\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.device_allow.len(), 1);
        assert_eq!(config.slice.device_allow[0], "/dev/zero r");
    }

    #[test]
    fn test_device_policy_auto() {
        let content = "[Slice]\nDevicePolicy=auto\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.device_policy, super::super::DevicePolicy::Auto);
    }

    #[test]
    fn test_device_policy_strict() {
        let content = "[Slice]\nDevicePolicy=strict\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.device_policy,
            super::super::DevicePolicy::Strict
        );
    }

    #[test]
    fn test_device_policy_closed() {
        let content = "[Slice]\nDevicePolicy=closed\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.device_policy,
            super::super::DevicePolicy::Closed
        );
    }

    #[test]
    fn test_ip_address_allow() {
        let content = "[Slice]\nIPAddressAllow=10.0.0.0/8 192.168.0.0/16\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.ip_address_allow.len(), 2);
        assert_eq!(config.slice.ip_address_allow[0], "10.0.0.0/8");
        assert_eq!(config.slice.ip_address_allow[1], "192.168.0.0/16");
    }

    #[test]
    fn test_ip_address_deny() {
        let content = "[Slice]\nIPAddressDeny=any\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.ip_address_deny, vec!["any"]);
    }

    #[test]
    fn test_ip_address_allow_reset() {
        let content =
            "[Slice]\nIPAddressAllow=10.0.0.0/8\nIPAddressAllow=\nIPAddressAllow=127.0.0.0/8\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.ip_address_allow.len(), 1);
        assert_eq!(config.slice.ip_address_allow[0], "127.0.0.0/8");
    }

    #[test]
    fn test_managed_oom_swap() {
        let content = "[Slice]\nManagedOOMSwap=kill\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.managed_oom_swap, Some("kill".to_owned()));
    }

    #[test]
    fn test_managed_oom_memory_pressure() {
        let content = "[Slice]\nManagedOOMMemoryPressure=kill\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.managed_oom_memory_pressure,
            Some("kill".to_owned())
        );
    }

    #[test]
    fn test_managed_oom_memory_pressure_limit() {
        let content = "[Slice]\nManagedOOMMemoryPressureLimit=50%\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.managed_oom_memory_pressure_limit,
            Some("50%".to_owned())
        );
    }

    #[test]
    fn test_managed_oom_preference() {
        let content = "[Slice]\nManagedOOMPreference=avoid\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.managed_oom_preference,
            Some("avoid".to_owned())
        );
    }

    #[test]
    fn test_memory_pressure_watch() {
        let content = "[Slice]\nMemoryPressureWatch=on\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.memory_pressure_watch,
            super::super::MemoryPressureWatch::On
        );
    }

    #[test]
    fn test_memory_pressure_watch_off() {
        let content = "[Slice]\nMemoryPressureWatch=off\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.memory_pressure_watch,
            super::super::MemoryPressureWatch::Off
        );
    }

    #[test]
    fn test_memory_pressure_watch_auto() {
        let content = "[Slice]\nMemoryPressureWatch=auto\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.memory_pressure_watch,
            super::super::MemoryPressureWatch::Auto
        );
    }

    #[test]
    fn test_combined_resource_control() {
        let content = "\
[Unit]
Description=Test Resource Control Slice

[Slice]
MemoryMax=2G
MemoryHigh=1G
MemoryLow=256M
CPUWeight=200
CPUQuota=50%
IOWeight=100
TasksMax=1024
Delegate=yes
CPUAccounting=yes
MemoryAccounting=yes
IOAccounting=yes
TasksAccounting=yes
DevicePolicy=closed
DeviceAllow=/dev/null rw
IPAddressDeny=any
IPAddressAllow=127.0.0.0/8
";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.memory_max.is_some());
        assert!(config.slice.memory_high.is_some());
        assert!(config.slice.memory_low.is_some());
        assert_eq!(config.slice.cpu_weight, Some(200));
        assert_eq!(config.slice.cpu_quota, Some(50));
        assert_eq!(config.slice.io_weight, Some(100));
        assert_eq!(
            config.slice.tasks_max,
            Some(super::super::TasksMax::Value(1024))
        );
        assert_eq!(config.slice.delegate, super::super::Delegate::Yes);
        assert_eq!(config.slice.cpu_accounting, Some(true));
        assert_eq!(config.slice.memory_accounting, Some(true));
        assert_eq!(config.slice.io_accounting, Some(true));
        assert_eq!(config.slice.tasks_accounting, Some(true));
        assert_eq!(
            config.slice.device_policy,
            super::super::DevicePolicy::Closed
        );
        assert_eq!(config.slice.device_allow.len(), 1);
        assert_eq!(config.slice.ip_address_deny, vec!["any"]);
        assert_eq!(config.slice.ip_address_allow, vec!["127.0.0.0/8"]);
    }

    #[test]
    fn test_unknown_key_ignored() {
        let content = "[Slice]\nSomeUnknownKey=value\n";
        let config = parse_slice_from_str(content).unwrap();
        // Should parse without error — unknown keys are traced and ignored
        assert!(config.slice.memory_max.is_none());
    }

    #[test]
    fn test_vendor_extension_ignored() {
        let content = "[Slice]\nX-CustomVendor=something\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.memory_max.is_none());
    }

    #[test]
    fn test_vendor_extension_section_ignored() {
        let content = "[X-Custom]\nFoo=bar\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.memory_max.is_none());
    }

    #[test]
    fn test_io_device_weight() {
        let content = "[Slice]\nIODeviceWeight=/dev/sda 200\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.io_device_weight.len(), 1);
        assert_eq!(config.slice.io_device_weight[0].device, "/dev/sda");
    }

    #[test]
    fn test_io_device_weight_reset() {
        let content = "[Slice]\nIODeviceWeight=/dev/sda 200\nIODeviceWeight=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.io_device_weight.len(), 0);
    }

    #[test]
    fn test_io_read_bandwidth_max() {
        let content = "[Slice]\nIOReadBandwidthMax=/dev/sda 10M\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.io_read_bandwidth_max.len(), 1);
    }

    #[test]
    fn test_io_write_bandwidth_max() {
        let content = "[Slice]\nIOWriteBandwidthMax=/dev/sda 5M\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.io_write_bandwidth_max.len(), 1);
    }

    #[test]
    fn test_io_read_iops_max() {
        let content = "[Slice]\nIOReadIOPSMax=/dev/sda 1000\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.io_read_iops_max.len(), 1);
    }

    #[test]
    fn test_io_write_iops_max() {
        let content = "[Slice]\nIOWriteIOPSMax=/dev/sda 500\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.io_write_iops_max.len(), 1);
    }

    #[test]
    fn test_empty_values_clear() {
        let content = "\
[Slice]
MemoryMax=1G
MemoryMax=
CPUWeight=200
CPUWeight=
IOWeight=100
IOWeight=
";
        let config = parse_slice_from_str(content).unwrap();
        // Empty assignment should clear / set to None
        assert!(config.slice.memory_max.is_none());
        assert!(config.slice.cpu_weight.is_none());
        assert!(config.slice.io_weight.is_none());
    }

    #[test]
    fn test_later_value_overrides() {
        let content = "\
[Slice]
MemoryMax=1G
MemoryMax=2G
CPUWeight=100
CPUWeight=500
";
        let config = parse_slice_from_str(content).unwrap();
        if let Some(super::super::MemoryLimit::Bytes(b)) = config.slice.memory_max {
            assert_eq!(b, 2 * 1024 * 1024 * 1024);
        } else {
            panic!("Expected MemoryLimit::Bytes(2G)");
        }
        assert_eq!(config.slice.cpu_weight, Some(500));
    }

    #[test]
    fn test_memory_swap_max_zero() {
        let content = "[Slice]\nMemorySwapMax=0\n";
        let config = parse_slice_from_str(content).unwrap();
        if let Some(super::super::MemoryLimit::Bytes(b)) = config.slice.memory_swap_max {
            assert_eq!(b, 0);
        } else {
            panic!("Expected MemoryLimit::Bytes(0)");
        }
    }

    #[test]
    fn test_device_policy_invalid() {
        let content = "[Slice]\nDevicePolicy=invalid\n";
        let result = parse_slice_from_str(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_memory_pressure_watch_invalid() {
        let content = "[Slice]\nMemoryPressureWatch=invalid\n";
        let result = parse_slice_from_str(content);
        assert!(result.is_err());
    }

    // =====================================================================
    // 18 new resource-control directives in [Slice] section
    // =====================================================================

    // --- CPUQuotaPeriodSec= ---

    #[test]
    fn test_cpu_quota_period_sec_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.cpu_quota_period_sec.is_none());
    }

    #[test]
    fn test_cpu_quota_period_sec_value() {
        let content = "[Slice]\nCPUQuotaPeriodSec=10\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.cpu_quota_period_sec,
            Some(super::super::Timeout::Duration(
                std::time::Duration::from_secs(10)
            ))
        );
    }

    #[test]
    fn test_cpu_quota_period_sec_empty() {
        let content = "[Slice]\nCPUQuotaPeriodSec=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.cpu_quota_period_sec.is_none());
    }

    // --- AllowedCPUs= ---

    #[test]
    fn test_allowed_cpus_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.allowed_cpus.is_none());
    }

    #[test]
    fn test_allowed_cpus_single() {
        let content = "[Slice]\nAllowedCPUs=0\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.allowed_cpus, Some("0".to_owned()));
    }

    #[test]
    fn test_allowed_cpus_range() {
        let content = "[Slice]\nAllowedCPUs=0-3\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.allowed_cpus, Some("0-3".to_owned()));
    }

    #[test]
    fn test_allowed_cpus_list() {
        let content = "[Slice]\nAllowedCPUs=0 2 4-7\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.allowed_cpus, Some("0 2 4-7".to_owned()));
    }

    #[test]
    fn test_allowed_cpus_empty_clears() {
        let content = "[Slice]\nAllowedCPUs=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.allowed_cpus.is_none());
    }

    // --- StartupAllowedCPUs= ---

    #[test]
    fn test_startup_allowed_cpus_value() {
        let content = "[Slice]\nStartupAllowedCPUs=0-1\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.startup_allowed_cpus, Some("0-1".to_owned()));
    }

    #[test]
    fn test_startup_allowed_cpus_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.startup_allowed_cpus.is_none());
    }

    // --- AllowedMemoryNodes= ---

    #[test]
    fn test_allowed_memory_nodes_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.allowed_memory_nodes.is_none());
    }

    #[test]
    fn test_allowed_memory_nodes_value() {
        let content = "[Slice]\nAllowedMemoryNodes=0-1\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.allowed_memory_nodes, Some("0-1".to_owned()));
    }

    #[test]
    fn test_allowed_memory_nodes_empty() {
        let content = "[Slice]\nAllowedMemoryNodes=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.allowed_memory_nodes.is_none());
    }

    // --- StartupAllowedMemoryNodes= ---

    #[test]
    fn test_startup_allowed_memory_nodes_value() {
        let content = "[Slice]\nStartupAllowedMemoryNodes=0\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.startup_allowed_memory_nodes,
            Some("0".to_owned())
        );
    }

    // --- DefaultMemoryMin= ---

    #[test]
    fn test_default_memory_min_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.default_memory_min.is_none());
    }

    #[test]
    fn test_default_memory_min_bytes() {
        let content = "[Slice]\nDefaultMemoryMin=64M\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.default_memory_min,
            Some(super::super::MemoryLimit::Bytes(64 * 1024 * 1024))
        );
    }

    #[test]
    fn test_default_memory_min_empty() {
        let content = "[Slice]\nDefaultMemoryMin=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.default_memory_min.is_none());
    }

    // --- DefaultMemoryLow= ---

    #[test]
    fn test_default_memory_low_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.default_memory_low.is_none());
    }

    #[test]
    fn test_default_memory_low_bytes() {
        let content = "[Slice]\nDefaultMemoryLow=128M\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.default_memory_low,
            Some(super::super::MemoryLimit::Bytes(128 * 1024 * 1024))
        );
    }

    #[test]
    fn test_default_memory_low_percentage() {
        let content = "[Slice]\nDefaultMemoryLow=25%\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.default_memory_low,
            Some(super::super::MemoryLimit::Percent(25))
        );
    }

    // --- MemoryZSwapMax= ---

    #[test]
    fn test_memory_zswap_max_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.memory_zswap_max.is_none());
    }

    #[test]
    fn test_memory_zswap_max_bytes() {
        let content = "[Slice]\nMemoryZSwapMax=256M\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.memory_zswap_max,
            Some(super::super::MemoryLimit::Bytes(256 * 1024 * 1024))
        );
    }

    #[test]
    fn test_memory_zswap_max_infinity() {
        let content = "[Slice]\nMemoryZSwapMax=infinity\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.memory_zswap_max,
            Some(super::super::MemoryLimit::Infinity)
        );
    }

    #[test]
    fn test_memory_zswap_max_percentage() {
        let content = "[Slice]\nMemoryZSwapMax=50%\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.memory_zswap_max,
            Some(super::super::MemoryLimit::Percent(50))
        );
    }

    #[test]
    fn test_memory_zswap_max_empty() {
        let content = "[Slice]\nMemoryZSwapMax=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.memory_zswap_max.is_none());
    }

    // --- IODeviceLatencyTargetSec= ---

    #[test]
    fn test_io_device_latency_target_sec_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.io_device_latency_target_sec.is_empty());
    }

    #[test]
    fn test_io_device_latency_target_sec_value() {
        let content = "[Slice]\nIODeviceLatencyTargetSec=/dev/sda 25ms\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.io_device_latency_target_sec,
            vec!["/dev/sda 25ms"]
        );
    }

    #[test]
    fn test_io_device_latency_target_sec_multiple() {
        let content = "[Slice]\nIODeviceLatencyTargetSec=/dev/sda 25ms\nIODeviceLatencyTargetSec=/dev/sdb 50ms\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.io_device_latency_target_sec.len(), 2);
        assert_eq!(
            config.slice.io_device_latency_target_sec[0],
            "/dev/sda 25ms"
        );
        assert_eq!(
            config.slice.io_device_latency_target_sec[1],
            "/dev/sdb 50ms"
        );
    }

    #[test]
    fn test_io_device_latency_target_sec_empty_resets() {
        let content =
            "[Slice]\nIODeviceLatencyTargetSec=/dev/sda 25ms\nIODeviceLatencyTargetSec=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.io_device_latency_target_sec.is_empty());
    }

    // --- DisableControllers= ---

    #[test]
    fn test_disable_controllers_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.disable_controllers.is_empty());
    }

    #[test]
    fn test_disable_controllers_single() {
        let content = "[Slice]\nDisableControllers=memory\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.disable_controllers, vec!["memory"]);
    }

    #[test]
    fn test_disable_controllers_multiple() {
        let content = "[Slice]\nDisableControllers=cpu memory io\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.disable_controllers,
            vec!["cpu", "memory", "io"]
        );
    }

    #[test]
    fn test_disable_controllers_empty_resets() {
        let content = "[Slice]\nDisableControllers=cpu\nDisableControllers=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.disable_controllers.is_empty());
    }

    #[test]
    fn test_disable_controllers_accumulates() {
        let content = "[Slice]\nDisableControllers=cpu\nDisableControllers=memory\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.disable_controllers, vec!["cpu", "memory"]);
    }

    // --- MemoryPressureThresholdSec= ---

    #[test]
    fn test_memory_pressure_threshold_sec_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.memory_pressure_threshold_sec.is_none());
    }

    #[test]
    fn test_memory_pressure_threshold_sec_value() {
        let content = "[Slice]\nMemoryPressureThresholdSec=200\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.memory_pressure_threshold_sec,
            Some(super::super::Timeout::Duration(
                std::time::Duration::from_secs(200)
            ))
        );
    }

    #[test]
    fn test_memory_pressure_threshold_sec_empty() {
        let content = "[Slice]\nMemoryPressureThresholdSec=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.memory_pressure_threshold_sec.is_none());
    }

    // --- IPIngressFilterPath= ---

    #[test]
    fn test_ip_ingress_filter_path_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.ip_ingress_filter_path.is_empty());
    }

    #[test]
    fn test_ip_ingress_filter_path_value() {
        let content = "[Slice]\nIPIngressFilterPath=/sys/fs/bpf/ingress\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.ip_ingress_filter_path,
            vec!["/sys/fs/bpf/ingress"]
        );
    }

    #[test]
    fn test_ip_ingress_filter_path_empty_resets() {
        let content = "[Slice]\nIPIngressFilterPath=/sys/fs/bpf/a\nIPIngressFilterPath=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.ip_ingress_filter_path.is_empty());
    }

    #[test]
    fn test_ip_ingress_filter_path_accumulates() {
        let content =
            "[Slice]\nIPIngressFilterPath=/sys/fs/bpf/a\nIPIngressFilterPath=/sys/fs/bpf/b\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.ip_ingress_filter_path.len(), 2);
    }

    // --- IPEgressFilterPath= ---

    #[test]
    fn test_ip_egress_filter_path_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.ip_egress_filter_path.is_empty());
    }

    #[test]
    fn test_ip_egress_filter_path_value() {
        let content = "[Slice]\nIPEgressFilterPath=/sys/fs/bpf/egress\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.ip_egress_filter_path,
            vec!["/sys/fs/bpf/egress"]
        );
    }

    #[test]
    fn test_ip_egress_filter_path_empty_resets() {
        let content = "[Slice]\nIPEgressFilterPath=/sys/fs/bpf/x\nIPEgressFilterPath=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.ip_egress_filter_path.is_empty());
    }

    // --- BPFProgram= ---

    #[test]
    fn test_bpf_program_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.bpf_program.is_empty());
    }

    #[test]
    fn test_bpf_program_value() {
        let content = "[Slice]\nBPFProgram=cgroup_skb/egress:/sys/fs/bpf/prog\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.bpf_program,
            vec!["cgroup_skb/egress:/sys/fs/bpf/prog"]
        );
    }

    #[test]
    fn test_bpf_program_empty_resets() {
        let content = "[Slice]\nBPFProgram=cgroup/a\nBPFProgram=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.bpf_program.is_empty());
    }

    #[test]
    fn test_bpf_program_accumulates() {
        let content = "[Slice]\nBPFProgram=cgroup/a\nBPFProgram=cgroup/b\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.bpf_program, vec!["cgroup/a", "cgroup/b"]);
    }

    // --- SocketBindAllow= ---

    #[test]
    fn test_socket_bind_allow_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.socket_bind_allow.is_empty());
    }

    #[test]
    fn test_socket_bind_allow_value() {
        let content = "[Slice]\nSocketBindAllow=tcp:8080\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.socket_bind_allow, vec!["tcp:8080"]);
    }

    #[test]
    fn test_socket_bind_allow_multiple() {
        let content = "[Slice]\nSocketBindAllow=tcp:8080\nSocketBindAllow=tcp:8443\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.socket_bind_allow.len(), 2);
    }

    #[test]
    fn test_socket_bind_allow_empty_resets() {
        let content = "[Slice]\nSocketBindAllow=tcp:80\nSocketBindAllow=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.socket_bind_allow.is_empty());
    }

    // --- SocketBindDeny= ---

    #[test]
    fn test_socket_bind_deny_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.socket_bind_deny.is_empty());
    }

    #[test]
    fn test_socket_bind_deny_value() {
        let content = "[Slice]\nSocketBindDeny=any\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.socket_bind_deny, vec!["any"]);
    }

    #[test]
    fn test_socket_bind_deny_empty_resets() {
        let content = "[Slice]\nSocketBindDeny=any\nSocketBindDeny=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.socket_bind_deny.is_empty());
    }

    // --- RestrictNetworkInterfaces= ---

    #[test]
    fn test_restrict_network_interfaces_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.restrict_network_interfaces.is_empty());
    }

    #[test]
    fn test_restrict_network_interfaces_allow() {
        let content = "[Slice]\nRestrictNetworkInterfaces=eth0 wlan0\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.restrict_network_interfaces,
            vec!["eth0", "wlan0"]
        );
    }

    #[test]
    fn test_restrict_network_interfaces_deny() {
        let content = "[Slice]\nRestrictNetworkInterfaces=~docker0 veth+\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.restrict_network_interfaces,
            vec!["~docker0", "veth+"]
        );
    }

    #[test]
    fn test_restrict_network_interfaces_empty_resets() {
        let content = "[Slice]\nRestrictNetworkInterfaces=eth0\nRestrictNetworkInterfaces=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.restrict_network_interfaces.is_empty());
    }

    #[test]
    fn test_restrict_network_interfaces_accumulates() {
        let content = "[Slice]\nRestrictNetworkInterfaces=eth0\nRestrictNetworkInterfaces=wlan0\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.restrict_network_interfaces,
            vec!["eth0", "wlan0"]
        );
    }

    // --- NFTSet= ---

    #[test]
    fn test_nft_set_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.nft_set.is_empty());
    }

    #[test]
    fn test_nft_set_value() {
        let content = "[Slice]\nNFTSet=inet:filter:cgroup_set\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.nft_set, vec!["inet:filter:cgroup_set"]);
    }

    #[test]
    fn test_nft_set_empty_resets() {
        let content = "[Slice]\nNFTSet=inet:a:b\nNFTSet=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.nft_set.is_empty());
    }

    #[test]
    fn test_nft_set_accumulates() {
        let content = "[Slice]\nNFTSet=inet:a:b\nNFTSet=inet:c:d\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(config.slice.nft_set, vec!["inet:a:b", "inet:c:d"]);
    }

    // --- DelegateSubgroup= ---

    #[test]
    fn test_delegate_subgroup_default() {
        let content = "[Slice]\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.delegate_subgroup.is_none());
    }

    #[test]
    fn test_delegate_subgroup_value() {
        let content = "[Slice]\nDelegateSubgroup=supervised\n";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.delegate_subgroup,
            Some("supervised".to_owned())
        );
    }

    #[test]
    fn test_delegate_subgroup_empty() {
        let content = "[Slice]\nDelegateSubgroup=\n";
        let config = parse_slice_from_str(content).unwrap();
        assert!(config.slice.delegate_subgroup.is_none());
    }

    // =====================================================================
    // Combined test for new resource-control directives
    // =====================================================================

    #[test]
    fn test_all_new_resource_control_directives() {
        let content = "\
[Slice]
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
DelegateSubgroup=supervised
";
        let config = parse_slice_from_str(content).unwrap();
        assert_eq!(
            config.slice.cpu_quota_period_sec,
            Some(super::super::Timeout::Duration(
                std::time::Duration::from_secs(5)
            ))
        );
        assert_eq!(config.slice.allowed_cpus, Some("0-3".to_owned()));
        assert_eq!(config.slice.startup_allowed_cpus, Some("0-1".to_owned()));
        assert_eq!(config.slice.allowed_memory_nodes, Some("0".to_owned()));
        assert_eq!(
            config.slice.startup_allowed_memory_nodes,
            Some("0".to_owned())
        );
        assert_eq!(
            config.slice.default_memory_min,
            Some(super::super::MemoryLimit::Bytes(32 * 1024 * 1024))
        );
        assert_eq!(
            config.slice.default_memory_low,
            Some(super::super::MemoryLimit::Bytes(64 * 1024 * 1024))
        );
        assert_eq!(
            config.slice.memory_zswap_max,
            Some(super::super::MemoryLimit::Bytes(512 * 1024 * 1024))
        );
        assert_eq!(
            config.slice.io_device_latency_target_sec,
            vec!["/dev/sda 10ms"]
        );
        assert_eq!(config.slice.disable_controllers, vec!["cpu", "io"]);
        assert_eq!(
            config.slice.memory_pressure_threshold_sec,
            Some(super::super::Timeout::Duration(
                std::time::Duration::from_secs(100)
            ))
        );
        assert_eq!(config.slice.ip_ingress_filter_path, vec!["/sys/fs/bpf/in"]);
        assert_eq!(config.slice.ip_egress_filter_path, vec!["/sys/fs/bpf/out"]);
        assert_eq!(
            config.slice.bpf_program,
            vec!["cgroup_skb/egress:/sys/fs/bpf/prog"]
        );
        assert_eq!(config.slice.socket_bind_allow, vec!["tcp:443"]);
        assert_eq!(config.slice.socket_bind_deny, vec!["any"]);
        assert_eq!(config.slice.restrict_network_interfaces, vec!["eth0"]);
        assert_eq!(config.slice.nft_set, vec!["inet:filter:service_set"]);
        assert_eq!(
            config.slice.delegate_subgroup,
            Some("supervised".to_owned())
        );
    }

    #[test]
    fn test_new_directives_coexist_with_existing_in_slice() {
        let content = "\
[Slice]
MemoryMax=2G
CPUWeight=200
CPUQuota=50%
IOWeight=100
TasksMax=4096
Delegate=yes
AllowedCPUs=0-3
MemoryZSwapMax=1G
DisableControllers=io
DelegateSubgroup=worker
";
        let config = parse_slice_from_str(content).unwrap();
        // Existing directives still work
        assert!(config.slice.memory_max.is_some());
        assert_eq!(config.slice.cpu_weight, Some(200));
        assert_eq!(config.slice.cpu_quota, Some(50));
        assert_eq!(config.slice.io_weight, Some(100));
        assert_eq!(
            config.slice.tasks_max,
            Some(super::super::TasksMax::Value(4096))
        );
        assert_eq!(config.slice.delegate, super::super::Delegate::Yes);
        // New directives work alongside
        assert_eq!(config.slice.allowed_cpus, Some("0-3".to_owned()));
        assert_eq!(
            config.slice.memory_zswap_max,
            Some(super::super::MemoryLimit::Bytes(1024 * 1024 * 1024))
        );
        assert_eq!(config.slice.disable_controllers, vec!["io"]);
        assert_eq!(config.slice.delegate_subgroup, Some("worker".to_owned()));
    }

    /// End-to-end test: parse → ParsedSliceSection → SliceConfig conversion
    /// preserves all 19 new resource-control fields.
    #[test]
    fn test_new_fields_survive_slice_config_conversion() {
        let content = "\
[Slice]
CPUQuotaPeriodSec=50ms
AllowedCPUs=0-3
StartupAllowedCPUs=0-1
AllowedMemoryNodes=0
StartupAllowedMemoryNodes=0-1
DefaultMemoryMin=64M
DefaultMemoryLow=128M
MemoryZSwapMax=512M
IODeviceLatencyTargetSec=/dev/sda 25ms
DisableControllers=hugetlb
MemoryPressureThresholdSec=2s
IPIngressFilterPath=/sys/fs/bpf/ingress
IPEgressFilterPath=/sys/fs/bpf/egress
BPFProgram=egress:/sys/fs/bpf/prog
SocketBindAllow=tcp:80
SocketBindDeny=any
RestrictNetworkInterfaces=eth0
NFTSet=inet:filter:addrs
DelegateSubgroup=supervisor
";
        let parsed = parse_slice_from_str(content).unwrap();
        // Convert ParsedSliceConfig → Unit → SliceConfig
        let unit: crate::units::Unit = parsed.try_into().unwrap();
        let slice_conf = match &unit.specific {
            crate::units::Specific::Slice(s) => &s.conf,
            _ => panic!("expected Slice variant"),
        };

        // Verify all 19 new fields survived the conversion
        assert!(
            slice_conf.cpu_quota_period_sec.is_some(),
            "CPUQuotaPeriodSec lost in conversion"
        );
        assert_eq!(slice_conf.allowed_cpus, Some("0-3".to_owned()));
        assert_eq!(slice_conf.startup_allowed_cpus, Some("0-1".to_owned()));
        assert_eq!(slice_conf.allowed_memory_nodes, Some("0".to_owned()));
        assert_eq!(
            slice_conf.startup_allowed_memory_nodes,
            Some("0-1".to_owned())
        );
        assert!(
            slice_conf.default_memory_min.is_some(),
            "DefaultMemoryMin lost in conversion"
        );
        assert!(
            slice_conf.default_memory_low.is_some(),
            "DefaultMemoryLow lost in conversion"
        );
        assert!(
            slice_conf.memory_zswap_max.is_some(),
            "MemoryZSwapMax lost in conversion"
        );
        assert_eq!(
            slice_conf.io_device_latency_target_sec,
            vec!["/dev/sda 25ms"]
        );
        assert_eq!(slice_conf.disable_controllers, vec!["hugetlb"]);
        assert!(
            slice_conf.memory_pressure_threshold_sec.is_some(),
            "MemoryPressureThresholdSec lost in conversion"
        );
        assert_eq!(
            slice_conf.ip_ingress_filter_path,
            vec!["/sys/fs/bpf/ingress"]
        );
        assert_eq!(slice_conf.ip_egress_filter_path, vec!["/sys/fs/bpf/egress"]);
        assert_eq!(slice_conf.bpf_program, vec!["egress:/sys/fs/bpf/prog"]);
        assert_eq!(slice_conf.socket_bind_allow, vec!["tcp:80"]);
        assert_eq!(slice_conf.socket_bind_deny, vec!["any"]);
        assert_eq!(slice_conf.restrict_network_interfaces, vec!["eth0"]);
        assert_eq!(slice_conf.nft_set, vec!["inet:filter:addrs"]);
        assert_eq!(slice_conf.delegate_subgroup, Some("supervisor".to_owned()));
    }

    /// Verify that systemctl show properties are populated from a parsed slice.
    #[test]
    fn test_slice_config_properties_from_parsed() {
        let content = "\
[Slice]
MemoryMax=4G
CPUWeight=500
CPUQuota=150%
IOWeight=200
TasksMax=1024
Delegate=yes
CPUAccounting=yes
AllowedCPUs=0-7
DelegateSubgroup=app
ManagedOOMSwap=kill
MemoryPressureWatch=on
DisableControllers=hugetlb
SocketBindAllow=tcp:8080
";
        let parsed = parse_slice_from_str(content).unwrap();
        let unit: crate::units::Unit = parsed.try_into().unwrap();

        let props = crate::control::unit_properties::collect_properties(&unit);

        assert_eq!(props.get("MemoryMax").unwrap(), "4294967296");
        assert_eq!(props.get("CPUWeight").unwrap(), "500");
        assert_eq!(props.get("CPUQuota").unwrap(), "150%");
        assert_eq!(props.get("IOWeight").unwrap(), "200");
        assert_eq!(props.get("TasksMax").unwrap(), "1024");
        assert_eq!(props.get("Delegate").unwrap(), "yes");
        assert_eq!(props.get("CPUAccounting").unwrap(), "yes");
        assert_eq!(props.get("AllowedCPUs").unwrap(), "0-7");
        assert_eq!(props.get("DelegateSubgroup").unwrap(), "app");
        assert_eq!(props.get("ManagedOOMSwap").unwrap(), "kill");
        assert_eq!(props.get("MemoryPressureWatch").unwrap(), "on");
        assert_eq!(props.get("DisableControllers").unwrap(), "hugetlb");
        assert_eq!(props.get("SocketBindAllow").unwrap(), "tcp:8080");
    }
}

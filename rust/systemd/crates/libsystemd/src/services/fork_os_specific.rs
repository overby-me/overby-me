use crate::units::PlatformSpecificServiceFields;
use crate::units::ServiceConfig;

#[cfg(feature = "cgroups")]
use crate::platform::cgroups;
#[cfg(feature = "cgroups")]
use crate::units::Delegate;
#[cfg(feature = "cgroups")]
use crate::units::TasksMax;
#[cfg(feature = "cgroups")]
use log::trace;

/// This is the place to do anything that is not standard unix but specific to one os. Like cgroups
pub fn pre_fork_os_specific(srvc: &ServiceConfig) -> Result<(), String> {
    #[cfg(feature = "cgroups")]
    {
        std::fs::create_dir_all(&srvc.platform_specific.cgroup_path).map_err(|e| {
            format!(
                "Couldnt create service cgroup ({:?}): {}",
                srvc.platform_specific.cgroup_path, e
            )
        })?;

        // When Delegate is enabled, chown the cgroup directory to the service user
        // so the service process can manage its own sub-cgroup hierarchy.
        if srvc.delegate != Delegate::No {
            let uid = super::start_service::resolve_uid(&srvc.exec_config.user)
                .map_err(|e| format!("Couldn't resolve user for cgroup delegation: {e}"))?;
            let uid = nix::unistd::Uid::from_raw(uid);
            let gid = super::start_service::resolve_gid(&srvc.exec_config.group)
                .map_err(|e| format!("Couldn't resolve group for cgroup delegation: {e}"))?;
            let gid = nix::unistd::Gid::from_raw(gid);
            trace!(
                "Delegating cgroup {:?} to uid={} gid={}",
                &srvc.platform_specific.cgroup_path, uid, gid
            );
            nix::unistd::chown(&srvc.platform_specific.cgroup_path, Some(uid), Some(gid)).map_err(
                |e| {
                    format!(
                        "Couldnt chown service cgroup ({:?}) to uid={} gid={}: {}",
                        srvc.platform_specific.cgroup_path, uid, gid, e
                    )
                },
            )?;
        }

        // Enable required cgroup controllers on the parent before writing limits.
        // We collect which controllers are needed based on the configured directives
        // and enable them all at once.
        {
            let mut needed_controllers: Vec<&str> = Vec::new();
            if srvc.tasks_max.is_some() || srvc.tasks_accounting == Some(true) {
                needed_controllers.push("pids");
            }
            if srvc.memory_min.is_some()
                || srvc.memory_low.is_some()
                || srvc.memory_high.is_some()
                || srvc.memory_max.is_some()
                || srvc.memory_swap_max.is_some()
                || srvc.memory_accounting == Some(true)
            {
                needed_controllers.push("memory");
            }
            if srvc.cpu_weight.is_some()
                || srvc.startup_cpu_weight.is_some()
                || srvc.cpu_quota.is_some()
                || srvc.cpu_accounting == Some(true)
            {
                needed_controllers.push("cpu");
            }
            if srvc.io_weight.is_some()
                || srvc.startup_io_weight.is_some()
                || !srvc.io_device_weight.is_empty()
                || !srvc.io_read_bandwidth_max.is_empty()
                || !srvc.io_write_bandwidth_max.is_empty()
                || !srvc.io_read_iops_max.is_empty()
                || !srvc.io_write_iops_max.is_empty()
                || srvc.io_accounting == Some(true)
            {
                needed_controllers.push("io");
            }
            if !needed_controllers.is_empty() {
                needed_controllers.dedup();
                if let Err(e) = cgroups::cgroup2::enable_controllers_on_parent(
                    &srvc.platform_specific.cgroup_path,
                    &needed_controllers,
                ) {
                    trace!(
                        "Could not enable cgroup controllers {:?} for {:?}: {}",
                        needed_controllers, &srvc.platform_specific.cgroup_path, e
                    );
                }
            }
        }

        // Apply TasksMax limit via the pids cgroup controller
        if let Some(ref tasks_max) = srvc.tasks_max {
            let pids_max_path = srvc.platform_specific.cgroup_path.join("pids.max");
            if pids_max_path.exists() || pids_max_path.parent().is_some_and(|p| p.exists()) {
                let value = match tasks_max {
                    TasksMax::Value(n) => n.to_string(),
                    TasksMax::Percent(pct) => {
                        // Read the system-wide pid limit and compute the percentage
                        let pid_max = std::fs::read_to_string("/proc/sys/kernel/pid_max")
                            .unwrap_or_else(|_| "32768".to_owned());
                        let pid_max: u64 = pid_max.trim().parse().unwrap_or(32768);
                        let limit = (pid_max * pct / 100).max(1);
                        limit.to_string()
                    }
                    TasksMax::Infinity => "max".to_owned(),
                };
                trace!(
                    "Setting TasksMax={} for cgroup {:?}",
                    value, &srvc.platform_specific.cgroup_path
                );
                if let Err(e) = std::fs::write(&pids_max_path, &value) {
                    trace!(
                        "Could not write pids.max for service cgroup ({:?}): {}",
                        pids_max_path, e
                    );
                }
            } else {
                trace!(
                    "pids.max not available for cgroup {:?}, skipping TasksMax",
                    &srvc.platform_specific.cgroup_path
                );
            }
        }

        // ── Memory controller limits ───────────────────────────────────────
        if let Some(ref limit) = srvc.memory_min {
            trace!(
                "Setting MemoryMin={:?} for cgroup {:?}",
                limit, &srvc.platform_specific.cgroup_path
            );
            if let Err(e) =
                cgroups::cgroup2::set_memory_min(&srvc.platform_specific.cgroup_path, limit)
            {
                trace!("Could not set memory.min: {}", e);
            }
        }
        if let Some(ref limit) = srvc.memory_low {
            trace!(
                "Setting MemoryLow={:?} for cgroup {:?}",
                limit, &srvc.platform_specific.cgroup_path
            );
            if let Err(e) =
                cgroups::cgroup2::set_memory_low(&srvc.platform_specific.cgroup_path, limit)
            {
                trace!("Could not set memory.low: {}", e);
            }
        }
        if let Some(ref limit) = srvc.memory_high {
            trace!(
                "Setting MemoryHigh={:?} for cgroup {:?}",
                limit, &srvc.platform_specific.cgroup_path
            );
            if let Err(e) =
                cgroups::cgroup2::set_memory_high(&srvc.platform_specific.cgroup_path, limit)
            {
                trace!("Could not set memory.high: {}", e);
            }
        }
        if let Some(ref limit) = srvc.memory_max {
            trace!(
                "Setting MemoryMax={:?} for cgroup {:?}",
                limit, &srvc.platform_specific.cgroup_path
            );
            if let Err(e) =
                cgroups::cgroup2::set_memory_max(&srvc.platform_specific.cgroup_path, limit)
            {
                trace!("Could not set memory.max: {}", e);
            }
        }
        if let Some(ref limit) = srvc.memory_swap_max {
            trace!(
                "Setting MemorySwapMax={:?} for cgroup {:?}",
                limit, &srvc.platform_specific.cgroup_path
            );
            if let Err(e) =
                cgroups::cgroup2::set_memory_swap_max(&srvc.platform_specific.cgroup_path, limit)
            {
                trace!("Could not set memory.swap.max: {}", e);
            }
        }

        // ── CPU controller limits ──────────────────────────────────────────
        if let Some(weight) = srvc.cpu_weight {
            trace!(
                "Setting CPUWeight={} for cgroup {:?}",
                weight, &srvc.platform_specific.cgroup_path
            );
            if let Err(e) =
                cgroups::cgroup2::set_cpu_weight(&srvc.platform_specific.cgroup_path, weight)
            {
                trace!("Could not set cpu.weight: {}", e);
            }
        }
        if let Some(quota) = srvc.cpu_quota {
            trace!(
                "Setting CPUQuota={}% for cgroup {:?}",
                quota, &srvc.platform_specific.cgroup_path
            );
            if let Err(e) =
                cgroups::cgroup2::set_cpu_quota(&srvc.platform_specific.cgroup_path, quota)
            {
                trace!("Could not set cpu.max: {}", e);
            }
        }

        // ── IO controller limits ───────────────────────────────────────────
        if let Some(weight) = srvc.io_weight {
            trace!(
                "Setting IOWeight={} for cgroup {:?}",
                weight, &srvc.platform_specific.cgroup_path
            );
            if let Err(e) =
                cgroups::cgroup2::set_io_weight(&srvc.platform_specific.cgroup_path, weight)
            {
                trace!("Could not set io.weight: {}", e);
            }
        }
        for entry in &srvc.io_device_weight {
            trace!(
                "Setting IODeviceWeight={} {} for cgroup {:?}",
                entry.device, entry.value, &srvc.platform_specific.cgroup_path
            );
            if let Err(e) = cgroups::cgroup2::set_io_device_weight(
                &srvc.platform_specific.cgroup_path,
                &entry.device,
                entry.value,
            ) {
                trace!("Could not set io.weight for {}: {}", entry.device, e);
            }
        }

        // Per-device bandwidth and IOPS limits — group by device to write
        // a single io.max line per device with all applicable limits.
        {
            type IoMaxLimits = (Option<u64>, Option<u64>, Option<u64>, Option<u64>);
            let mut io_max_devices: std::collections::HashMap<String, IoMaxLimits> =
                std::collections::HashMap::new();

            for entry in &srvc.io_read_bandwidth_max {
                io_max_devices
                    .entry(entry.device.clone())
                    .or_insert((None, None, None, None))
                    .0 = Some(entry.value);
            }
            for entry in &srvc.io_write_bandwidth_max {
                io_max_devices
                    .entry(entry.device.clone())
                    .or_insert((None, None, None, None))
                    .1 = Some(entry.value);
            }
            for entry in &srvc.io_read_iops_max {
                io_max_devices
                    .entry(entry.device.clone())
                    .or_insert((None, None, None, None))
                    .2 = Some(entry.value);
            }
            for entry in &srvc.io_write_iops_max {
                io_max_devices
                    .entry(entry.device.clone())
                    .or_insert((None, None, None, None))
                    .3 = Some(entry.value);
            }

            for (device, (rbps, wbps, riops, wiops)) in &io_max_devices {
                trace!(
                    "Setting io.max for {} rbps={:?} wbps={:?} riops={:?} wiops={:?} in cgroup {:?}",
                    device, rbps, wbps, riops, wiops, &srvc.platform_specific.cgroup_path
                );
                if let Err(e) = cgroups::cgroup2::set_io_max(
                    &srvc.platform_specific.cgroup_path,
                    device,
                    *rbps,
                    *wbps,
                    *riops,
                    *wiops,
                ) {
                    trace!("Could not set io.max for {}: {}", device, e);
                }
            }
        }
        // ── Device access policy (BPF device controller) ─────────────────
        {
            use crate::units::unit_parsing::DevicePolicy;
            if srvc.device_policy != DevicePolicy::Auto || !srvc.device_allow.is_empty() {
                trace!(
                    "Applying DevicePolicy={:?} with {} DeviceAllow rules for cgroup {:?}",
                    srvc.device_policy,
                    srvc.device_allow.len(),
                    &srvc.platform_specific.cgroup_path
                );
                match cgroups::bpf_devices::apply_device_policy(
                    &srvc.platform_specific.cgroup_path,
                    &srvc.device_policy,
                    &srvc.device_allow,
                ) {
                    Ok(()) => {
                        trace!(
                            "Device policy applied for cgroup {:?}",
                            &srvc.platform_specific.cgroup_path
                        );
                    }
                    Err(e) => {
                        trace!("Could not apply device policy: {}", e);
                    }
                }
            }
        }
    }
    let _ = srvc;
    Ok(())
}

pub fn post_fork_os_specific(conf: &PlatformSpecificServiceFields) -> Result<(), String> {
    #[cfg(feature = "cgroups")]
    {
        use log::trace;
        trace!("Move service to cgroup: {:?}", &conf.cgroup_path);
        cgroups::move_self_to_cgroup(&conf.cgroup_path)
            .map_err(|e| format!("postfork os specific: {}", e))?;
    }
    let _ = conf;
    Ok(())
}

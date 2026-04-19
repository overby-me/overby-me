//! Handle udev events forwarded by systemd-udevd via the control socket.
//!
//! For every device the kernel announces, udevd evaluates the rule set and
//! then sends a `udev-event` JSON-RPC notification to PID 1 with the event
//! action, full sysfs path, env properties (`SYSTEMD_ALIAS=`, `SYSTEMD_WANTS=`,
//! `SYSTEMD_READY=`), tag set and symlinks.  This module maps that
//! notification onto the service-manager's `.device` unit table:
//!
//! - On `add`/`change` with tag `systemd` (or any `SYSTEMD_ALIAS=` / symlink
//!   alias): create the primary device unit plus aliases and mark them as
//!   `Started(Plugged)`.  Dependencies are fired via the usual activation
//!   path, so units with `BindsTo=sys-subsystem-net-devices-eth0.device` are
//!   activated when the device appears.
//! - On `remove` / `unbind` / `offline`: mark the device units as `Stopped`.
//! - `SYSTEMD_WANTS=` adds a runtime `Wants=` edge so the listed units are
//!   pulled in whenever the device activates.
//!
//! The module keeps its own logic narrow on purpose: real udev→systemd
//! integration has quite a few corners (SYSTEMD_READY, ID_PROCESSING, alias
//! change semantics) that we add incrementally as tests need them.

use crate::control::UdevEventParams;
use crate::lock_ext::RwLockExt;
use crate::runtime_info::ArcMutRuntimeInfo;
use crate::units::status::{StatusStarted, StatusStopped, UnitStatus};
use crate::units::unit::Unit;
use crate::units::{UnitId, UnitIdKind};
use log::{debug, trace, warn};

/// Entry point for a `Command::UdevEvent` notification.
pub fn handle_udev_event(run_info: &ArcMutRuntimeInfo, params: &UdevEventParams) {
    trace!(
        "udev-event: action={} sysfs_path={} tags={:?} symlinks={:?}",
        params.action, params.sysfs_path, params.tags, params.symlinks
    );

    let unit_names = derive_unit_names(params);
    if unit_names.is_empty() {
        // Not a device we need to track in systemd: no tag `systemd`, no alias,
        // no devname, no symlinks.  Skip silently.
        trace!(
            "udev-event: no device units derived for sysfs_path={}; skipping",
            params.sysfs_path
        );
        return;
    }

    match params.action.as_str() {
        "add" | "change" | "bind" | "move" | "online" => {
            // While a device is still being processed by udevd (RUN= program
            // running), defer activation — upstream systemd waits for the
            // RUN block to finish before considering the device plugged, and
            // TEST-17-UDEV.device_is_processing asserts the associated
            // .device units remain inactive during this window.
            let is_processing = params
                .env
                .get("ID_PROCESSING")
                .map(|v| v == "1")
                .unwrap_or(false);
            // SYSTEMD_READY=0 explicitly marks a device as not-ready.  The
            // device unit is created so units can still reference it via
            // deps, but it's kept Stopped until a later change event with
            // SYSTEMD_READY=1 (or absent) flips it to active.
            let systemd_ready = params
                .env
                .get("SYSTEMD_READY")
                .map(|v| v != "0")
                .unwrap_or(true);
            // ID_RENAMING=1 is set by udev while a network interface is
            // renaming.  Treat it like SYSTEMD_READY=0 — device is
            // present but not usable, so BindsTo= dependents should see
            // it as inactive.  TEST-17-UDEV.rename-netif uses this to
            // drive .device unit deactivation during renames.
            let is_renaming = params
                .env
                .get("ID_RENAMING")
                .map(|v| v == "1")
                .unwrap_or(false);
            if is_processing || !systemd_ready || is_renaming {
                trace!(
                    "udev-event: {} deferred (ID_PROCESSING={}, SYSTEMD_READY={}, ID_RENAMING={}); placeholder units only",
                    params.sysfs_path, is_processing, systemd_ready, is_renaming
                );
                ensure_device_units_exist(run_info, params, &unit_names);
                // When SYSTEMD_READY flipped to 0 or the device entered
                // renaming, transition any previously-plugged units
                // back to Stopped so BindsTo= dependents deactivate.
                if !systemd_ready || is_renaming {
                    apply_device_inactive(run_info, &unit_names);
                }
            } else {
                apply_device_active(run_info, params, &unit_names);
            }
        }
        "remove" | "unbind" | "offline" => {
            apply_device_inactive(run_info, &unit_names);
        }
        other => {
            debug!("udev-event: ignoring unknown action '{}'", other);
        }
    }
}

/// Create placeholder device units without activating them — used during
/// ID_PROCESSING=1 so `systemctl show` / D-Bus object lookup work, but the
/// units stay `Stopped` so `BindsTo=` dependents aren't fired prematurely.
fn ensure_device_units_exist(
    run_info: &ArcMutRuntimeInfo,
    params: &UdevEventParams,
    unit_names: &[String],
) {
    let mut ri = run_info.write_poisoned();
    for name in unit_names {
        let id = UnitId {
            kind: UnitIdKind::Device,
            name: name.clone(),
        };
        if !ri.unit_table.contains_key(&id) {
            match build_device_unit(name, params) {
                Ok(unit) => {
                    trace!("udev-event: creating placeholder device unit {}", name);
                    // Use insert_new_unit_lenient so bidirectional
                    // dependency edges (wanted_by, bound_by, etc.) are
                    // populated against already-present units.  A plain
                    // `unit_table.insert` would leave reverse edges
                    // blank, breaking `systemctl show -p WantedBy
                    // foo.service` on units pulled in via SYSTEMD_WANTS.
                    crate::units::insert_new_unit_lenient(unit, &mut ri);
                }
                Err(e) => {
                    warn!("udev-event: failed to build device unit {}: {}", name, e);
                }
            }
        }
    }
}

/// Derive the set of `.device` unit names for this udev event.
///
/// The primary name comes from `sysfs_path`.  Additional names come from:
/// - a subsystem-based alias (`sys-subsystem-<subsys>-devices-<KERNEL>.device`)
/// - any `/dev` symlinks (`SYMLINK+=…`)
/// - any `SYSTEMD_ALIAS=` env values (each is a path to escape)
/// - the primary `/dev` device node (`devname`)
pub fn derive_unit_names(params: &UdevEventParams) -> Vec<String> {
    let tagged_systemd = params.tags.iter().any(|t| t == "systemd");
    let has_alias = params.env.contains_key("SYSTEMD_ALIAS");
    let has_symlink = !params.symlinks.is_empty();

    // Don't create units for devices that aren't relevant to the service
    // manager.  Real systemd only creates `.device` units for devices with
    // `TAG=systemd`, `SYSTEMD_ALIAS=`, or explicit unit-file references.
    if !tagged_systemd && !has_alias && !has_symlink {
        return Vec::new();
    }

    let mut names: Vec<String> = Vec::new();

    // Primary sysfs-derived name.
    if !params.sysfs_path.is_empty() {
        names.push(path_to_device_unit_name(&params.sysfs_path));
    }

    // Subsystem-based friendly alias, e.g. /sys/devices/virtual/net/eth0 →
    // sys-subsystem-net-devices-eth0.device.  Both the subsystem and the
    // kernel name are run through unit_name_escape so that
    // SUBSYSTEM=net KERNEL=test\x2dnetif\x2dfoo produces the properly
    // escaped `sys-subsystem-net-devices-test\x2dnetif\x2dfoo.device`.
    if !params.subsystem.is_empty()
        && let Some(kernel_name) = params.sysfs_path.rsplit('/').next()
        && !kernel_name.is_empty()
    {
        let subsys_esc = crate::unit_name::unit_name_escape(&params.subsystem);
        let kernel_esc = crate::unit_name::unit_name_escape(kernel_name);
        let alias = format!(
            "sys-subsystem-{}-devices-{}.device",
            subsys_esc, kernel_esc
        );
        if !names.contains(&alias) {
            names.push(alias);
        }
    }

    // /dev/<node>.device for the main device node.
    if !params.devname.is_empty() {
        let n = path_to_device_unit_name(&params.devname);
        if !names.contains(&n) {
            names.push(n);
        }
    }

    // /dev/... symlinks — each creates its own alias device unit.
    for link in &params.symlinks {
        let link_path = if link.starts_with('/') {
            link.clone()
        } else {
            format!("/dev/{}", link)
        };
        let n = path_to_device_unit_name(&link_path);
        if !names.contains(&n) {
            names.push(n);
        }
    }

    // SYSTEMD_ALIAS= env values (space- or newline-separated list of paths).
    if let Some(alias_val) = params.env.get("SYSTEMD_ALIAS") {
        for alias in alias_val.split_whitespace() {
            if alias.is_empty() {
                continue;
            }
            let n = path_to_device_unit_name(alias);
            if !names.contains(&n) {
                names.push(n);
            }
        }
    }

    names
}

/// Convert a sysfs / device path into a `.device` unit name.
///
/// Uses the systemd `unit_name_path_escape` convention: `/` separates
/// path components as `-`, and any character outside `[a-zA-Z0-9:_.]`
/// is escaped as `\xNN`.  This matches the output of
/// `systemd-escape --path --suffix=device …` which
/// TEST-17-UDEV.SYSTEMD_WANTS-escape uses to construct expected unit
/// names.
pub fn path_to_device_unit_name(path: &str) -> String {
    format!("{}.device", crate::unit_name::unit_name_path_escape(path))
}

/// Mark the listed device units as plugged (started).  Creates the unit
/// if not yet present in the unit table.  After flipping the state,
/// triggers activation of any `Wants=` / `Requires=` targets (e.g. from
/// `SYSTEMD_WANTS=` or explicit `.device` unit files) so dependents
/// registered via `BindsTo=<name>.device` actually start.
fn apply_device_active(
    run_info: &ArcMutRuntimeInfo,
    params: &UdevEventParams,
    unit_names: &[String],
) {
    // First, grab a write lock on the unit table and create any missing
    // device units.  Keep the lock scope narrow so we don't deadlock the
    // activation path.
    {
        let mut ri = run_info.write_poisoned();
        for name in unit_names {
            let id = UnitId {
                kind: UnitIdKind::Device,
                name: name.clone(),
            };
            if !ri.unit_table.contains_key(&id) {
                match build_device_unit(name, params) {
                    Ok(unit) => {
                        trace!("udev-event: creating device unit {}", name);
                        // Use insert_new_unit_lenient so the unit's
                        // Wants= (from SYSTEMD_WANTS=) propagates a
                        // corresponding `wanted_by` edge into the
                        // target.  `systemctl show -p WantedBy` on the
                        // target unit will then include this device.
                        crate::units::insert_new_unit_lenient(unit, &mut ri);
                    }
                    Err(e) => {
                        warn!("udev-event: failed to build device unit {}: {}", name, e);
                    }
                }
            }
        }
    }

    // Then flip each unit's status to Started(Plugged).  This is done
    // with the read lock held, via the unit's own status RwLock.
    {
        let ri = run_info.read_poisoned();
        for name in unit_names {
            let id = UnitId {
                kind: UnitIdKind::Device,
                name: name.clone(),
            };
            if let Some(unit) = ri.unit_table.get(&id) {
                let mut status = unit.common.status.write_poisoned();
                if !matches!(&*status, UnitStatus::Started(_)) {
                    trace!("udev-event: marking {} as plugged", name);
                    *status = UnitStatus::Started(StatusStarted::Running);
                }
            }
        }
    }

    // Now pull in SYSTEMD_WANTS= / .device-file-declared Wants= targets.
    // We spawn this on a thread so the control-socket handler doesn't
    // block if the activation subgraph is slow (e.g. waiting on a
    // Type=notify service).
    for name in unit_names {
        let id = UnitId {
            kind: UnitIdKind::Device,
            name: name.clone(),
        };
        let run_info = run_info.clone();
        let unit_name = name.clone();
        std::thread::spawn(move || {
            let deps_to_activate: Vec<UnitId> = {
                let ri = run_info.read_poisoned();
                match ri.unit_table.get(&id) {
                    Some(unit) => unit
                        .common
                        .dependencies
                        .wants
                        .iter()
                        .chain(unit.common.dependencies.requires.iter())
                        .cloned()
                        .collect(),
                    None => Vec::new(),
                }
            };
            for dep in deps_to_activate {
                trace!(
                    "udev-event: activating dependency {} of device unit {}",
                    dep.name, unit_name
                );
                let errors = crate::units::activate_needed_units_with_source(
                    dep.clone(),
                    run_info.clone(),
                    crate::units::ActivationSource::Regular,
                );
                for err in errors {
                    warn!(
                        "udev-event: failed to activate dependency {}: {}",
                        dep.name, err.reason
                    );
                }
            }
        });
    }
}

/// Mark the listed device units as inactive (stopped).  Does NOT remove
/// them from the unit table — systemd keeps the entry around for
/// reference, but transitions any `BindsTo=` dependents to stopped too.
fn apply_device_inactive(run_info: &ArcMutRuntimeInfo, unit_names: &[String]) {
    let ri = run_info.read_poisoned();
    for name in unit_names {
        let id = UnitId {
            kind: UnitIdKind::Device,
            name: name.clone(),
        };
        if let Some(unit) = ri.unit_table.get(&id) {
            let mut status = unit.common.status.write_poisoned();
            if !matches!(
                &*status,
                UnitStatus::Stopped(_, _) | UnitStatus::NeverStarted
            ) {
                trace!("udev-event: marking {} as stopped", name);
                *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, Vec::new());
            }
        }
    }
}

/// Construct a fresh device Unit struct with udev-derived metadata applied
/// to `Wants=` (from SYSTEMD_WANTS=).
fn build_device_unit(name: &str, params: &UdevEventParams) -> Result<Unit, String> {
    let wants: Vec<String> = params
        .env
        .get("SYSTEMD_WANTS")
        .map(|v| {
            v.split_whitespace()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    let description = if !params.devname.is_empty() {
        format!("Device for {}", params.devname)
    } else {
        format!("Device {}", params.sysfs_path)
    };

    crate::units::from_parsed_config::create_device_unit(
        name,
        Some(params.sysfs_path.clone()),
        &wants,
        Some(description),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_params(
        action: &str,
        sysfs_path: &str,
        subsystem: &str,
        tags: &[&str],
        symlinks: &[&str],
    ) -> UdevEventParams {
        UdevEventParams {
            action: action.to_owned(),
            sysfs_path: sysfs_path.to_owned(),
            devname: String::new(),
            subsystem: subsystem.to_owned(),
            env: HashMap::new(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            symlinks: symlinks.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn test_path_to_device_unit_name_basic() {
        assert_eq!(
            path_to_device_unit_name("/sys/devices/virtual/net/eth0"),
            "sys-devices-virtual-net-eth0.device"
        );
        assert_eq!(path_to_device_unit_name("/dev/sda1"), "dev-sda1.device");
        assert_eq!(path_to_device_unit_name("/"), "-.device");
    }

    #[test]
    fn test_derive_unit_names_untagged_returns_empty() {
        let p = make_params("add", "/sys/devices/virtual/net/eth0", "net", &[], &[]);
        assert!(derive_unit_names(&p).is_empty());
    }

    #[test]
    fn test_derive_unit_names_with_systemd_tag() {
        let p = make_params(
            "add",
            "/sys/devices/virtual/net/eth0",
            "net",
            &["systemd"],
            &[],
        );
        let names = derive_unit_names(&p);
        assert!(names.contains(&"sys-devices-virtual-net-eth0.device".to_owned()));
        assert!(names.contains(&"sys-subsystem-net-devices-eth0.device".to_owned()));
    }

    #[test]
    fn test_derive_unit_names_with_alias_env() {
        let mut p = make_params(
            "add",
            "/sys/devices/virtual/mem/null",
            "mem",
            &[],
            &[],
        );
        p.env.insert(
            "SYSTEMD_ALIAS".to_owned(),
            "/sys/test/alias-to-null-on-add".to_owned(),
        );
        let names = derive_unit_names(&p);
        // unit_name_path_escape turns each literal `-` into `\x2d`, so
        // /sys/test/alias-to-null-on-add →
        // sys-test-alias\x2dto\x2dnull\x2don\x2dadd.device
        assert!(
            names.contains(&r"sys-test-alias\x2dto\x2dnull\x2don\x2dadd.device".to_owned()),
            "expected escaped alias in {names:?}"
        );
    }

    #[test]
    fn test_derive_unit_names_with_symlink() {
        let p = make_params(
            "add",
            "/sys/devices/virtual/mem/null",
            "mem",
            &["systemd"],
            &["/dev/test/symlink-to-null-on-add"],
        );
        let names = derive_unit_names(&p);
        assert!(names.iter().any(|n| n.contains("symlink")));
    }

    #[test]
    fn test_derive_unit_names_devname_included() {
        let mut p = make_params(
            "add",
            "/sys/devices/pci0000:00/sda/sda1",
            "block",
            &["systemd"],
            &[],
        );
        p.devname = "/dev/sda1".to_owned();
        let names = derive_unit_names(&p);
        assert!(names.contains(&"dev-sda1.device".to_owned()));
    }

    // -----------------------------------------------------------------------
    // handle_udev_event integration tests
    //
    // Construct a minimal ArcMutRuntimeInfo, fire a synthetic
    // UdevEventParams at handle_udev_event, and assert the unit_table
    // state afterwards.  These bypass the Wants=-activation background
    // thread (which needs a full service-manager runtime) and focus on
    // the synchronous unit-creation + status-transition logic.
    // -----------------------------------------------------------------------

    use crate::runtime_info::{ArcMutRuntimeInfo, PidTable, RuntimeInfo, UnitTable};
    use std::sync::{Arc, Mutex, RwLock};

    fn make_test_runtime() -> ArcMutRuntimeInfo {
        Arc::new(RwLock::new(RuntimeInfo {
            config: crate::config::Config {
                notification_sockets_dir: "/tmp".into(),
                target_unit: "".into(),
                unit_dirs: vec![],
                self_path: std::path::PathBuf::from("./rust-systemd"),
            },
            fd_store: RwLock::new(crate::fd_store::FDStore::default()),
            pid_table: Arc::new(Mutex::new(PidTable::default())),
            unit_table: UnitTable::default(),
            stdout_eventfd: crate::platform::make_event_fd().unwrap(),
            stderr_eventfd: crate::platform::make_event_fd().unwrap(),
            notification_eventfd: crate::platform::make_event_fd().unwrap(),
            socket_activation_eventfd: crate::platform::make_event_fd().unwrap(),
            pending_activations: Arc::new(Mutex::new(std::collections::HashSet::new())),
            manager_environment: Arc::new(Mutex::new(std::collections::HashMap::new())),
            unit_markers: Arc::new(Mutex::new(std::collections::HashMap::new())),
            transactions_with_cycle: Arc::new(Mutex::new(Vec::new())),
            units_in_cycles: Arc::new(Mutex::new(std::collections::HashSet::new())),
        }))
    }

    fn unit_status(run_info: &ArcMutRuntimeInfo, name: &str) -> Option<UnitStatus> {
        let ri = run_info.read().unwrap();
        ri.unit_table.values().find_map(|u| {
            if u.id.name == name {
                Some(u.common.status.read().unwrap().clone())
            } else {
                None
            }
        })
    }

    fn unit_exists(run_info: &ArcMutRuntimeInfo, name: &str) -> bool {
        let ri = run_info.read().unwrap();
        ri.unit_table.values().any(|u| u.id.name == name)
    }

    #[test]
    fn test_handle_udev_event_add_untagged_noop() {
        let ri = make_test_runtime();
        let params = make_params("add", "/sys/devices/virtual/net/eth0", "net", &[], &[]);
        handle_udev_event(&ri, &params);
        // No unit should have been created for an untagged device.
        let ri_guard = ri.read().unwrap();
        assert!(ri_guard.unit_table.is_empty());
    }

    #[test]
    fn test_handle_udev_event_add_tagged_creates_and_activates() {
        let ri = make_test_runtime();
        let params = make_params(
            "add",
            "/sys/devices/virtual/net/eth0",
            "net",
            &["systemd"],
            &[],
        );
        handle_udev_event(&ri, &params);
        assert!(unit_exists(&ri, "sys-devices-virtual-net-eth0.device"));
        assert!(unit_exists(&ri, "sys-subsystem-net-devices-eth0.device"));
        let s = unit_status(&ri, "sys-devices-virtual-net-eth0.device").unwrap();
        assert!(
            matches!(s, UnitStatus::Started(_)),
            "expected Started, got {s:?}"
        );
    }

    #[test]
    fn test_handle_udev_event_id_processing_defers_activation() {
        let ri = make_test_runtime();
        let mut params = make_params(
            "add",
            "/sys/devices/virtual/net/eth0",
            "net",
            &["systemd"],
            &[],
        );
        params.env.insert("ID_PROCESSING".into(), "1".into());
        handle_udev_event(&ri, &params);
        assert!(unit_exists(&ri, "sys-devices-virtual-net-eth0.device"));
        let s = unit_status(&ri, "sys-devices-virtual-net-eth0.device").unwrap();
        assert!(
            !matches!(s, UnitStatus::Started(_)),
            "expected placeholder (non-Started), got {s:?}"
        );
    }

    #[test]
    fn test_handle_udev_event_systemd_ready_zero_keeps_inactive() {
        let ri = make_test_runtime();
        // First: add with SYSTEMD_READY=0 — placeholder only.
        let mut params = make_params(
            "add",
            "/sys/devices/virtual/mem/foo",
            "mem",
            &["systemd"],
            &[],
        );
        params.env.insert("SYSTEMD_READY".into(), "0".into());
        handle_udev_event(&ri, &params);
        let s = unit_status(&ri, "sys-devices-virtual-mem-foo.device").unwrap();
        assert!(!matches!(s, UnitStatus::Started(_)));
    }

    #[test]
    fn test_handle_udev_event_remove_deactivates() {
        let ri = make_test_runtime();
        // add then remove.
        let add_params = make_params(
            "add",
            "/sys/devices/virtual/net/eth0",
            "net",
            &["systemd"],
            &[],
        );
        handle_udev_event(&ri, &add_params);
        let s_before = unit_status(&ri, "sys-devices-virtual-net-eth0.device").unwrap();
        assert!(matches!(s_before, UnitStatus::Started(_)));

        let remove_params = make_params(
            "remove",
            "/sys/devices/virtual/net/eth0",
            "net",
            &["systemd"],
            &[],
        );
        handle_udev_event(&ri, &remove_params);
        let s_after = unit_status(&ri, "sys-devices-virtual-net-eth0.device").unwrap();
        assert!(
            matches!(s_after, UnitStatus::Stopped(_, _)),
            "expected Stopped, got {s_after:?}"
        );
    }

    #[test]
    fn test_handle_udev_event_id_renaming_flips_to_inactive() {
        let ri = make_test_runtime();
        // add: active.
        let add_params = make_params(
            "add",
            "/sys/devices/virtual/net/hoge",
            "net",
            &["systemd"],
            &[],
        );
        handle_udev_event(&ri, &add_params);
        let s1 = unit_status(&ri, "sys-devices-virtual-net-hoge.device").unwrap();
        assert!(matches!(s1, UnitStatus::Started(_)));

        // change with ID_RENAMING=1: should flip to inactive (per
        // TEST-17-UDEV.rename-netif).
        let mut rename_params = make_params(
            "change",
            "/sys/devices/virtual/net/hoge",
            "net",
            &["systemd"],
            &[],
        );
        rename_params.env.insert("ID_RENAMING".into(), "1".into());
        handle_udev_event(&ri, &rename_params);
        let s2 = unit_status(&ri, "sys-devices-virtual-net-hoge.device").unwrap();
        assert!(
            !matches!(s2, UnitStatus::Started(_)),
            "ID_RENAMING=1 must demote to non-Started, got {s2:?}"
        );

        // subsequent move without ID_RENAMING restores active state.
        let restore_params = make_params(
            "move",
            "/sys/devices/virtual/net/hoge",
            "net",
            &["systemd"],
            &[],
        );
        handle_udev_event(&ri, &restore_params);
        let s3 = unit_status(&ri, "sys-devices-virtual-net-hoge.device").unwrap();
        assert!(
            matches!(s3, UnitStatus::Started(_)),
            "after clear ID_RENAMING, expected Started, got {s3:?}"
        );
    }

    #[test]
    fn test_handle_udev_event_systemd_wants_populates_dep_edges() {
        let ri = make_test_runtime();
        let mut params = make_params(
            "add",
            "/sys/devices/virtual/net/eth0",
            "net",
            &["systemd"],
            &[],
        );
        params
            .env
            .insert("SYSTEMD_WANTS".into(), "foo.service bar.service".into());
        handle_udev_event(&ri, &params);

        let ri_guard = ri.read().unwrap();
        let device_unit = ri_guard
            .unit_table
            .values()
            .find(|u| u.id.name == "sys-devices-virtual-net-eth0.device")
            .expect("device unit not created");
        let wants: Vec<&str> = device_unit
            .common
            .dependencies
            .wants
            .iter()
            .map(|id| id.name.as_str())
            .collect();
        assert!(
            wants.contains(&"foo.service"),
            "Wants missing foo.service: {wants:?}"
        );
        assert!(
            wants.contains(&"bar.service"),
            "Wants missing bar.service: {wants:?}"
        );
    }

    #[test]
    fn test_handle_udev_event_symlink_creates_alias_unit() {
        let ri = make_test_runtime();
        let params = make_params(
            "add",
            "/sys/devices/virtual/mem/null",
            "mem",
            &["systemd"],
            &["/dev/disk/by-uuid/abcdef"],
        );
        handle_udev_event(&ri, &params);
        // Primary sysfs-derived name.
        assert!(unit_exists(&ri, "sys-devices-virtual-mem-null.device"));
        // Symlink-derived alias (unit_name_path_escape produces
        // dev-disk-by\x2duuid-abcdef.device from /dev/disk/by-uuid/abcdef).
        let alias_name = r"dev-disk-by\x2duuid-abcdef.device";
        assert!(
            unit_exists(&ri, alias_name),
            "symlink alias unit {alias_name} not created"
        );
    }

    #[test]
    fn test_handle_udev_event_repeat_add_idempotent() {
        let ri = make_test_runtime();
        let params = make_params(
            "add",
            "/sys/devices/virtual/net/eth0",
            "net",
            &["systemd"],
            &[],
        );
        handle_udev_event(&ri, &params);
        let count_before = ri.read().unwrap().unit_table.len();

        // Second identical event: should be a no-op on unit_table.
        handle_udev_event(&ri, &params);
        let count_after = ri.read().unwrap().unit_table.len();
        assert_eq!(
            count_before, count_after,
            "repeat add duplicated units in unit_table"
        );
    }
}

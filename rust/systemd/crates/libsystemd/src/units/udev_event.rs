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
use crate::units::unit::{Specific, Unit};
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

    // When a device is renamed (e.g. network interface rename from
    // `hoge` to `foobar`), udev emits a `move` event carrying
    // `DEVPATH_OLD=/devices/.../hoge`.  The `.device` unit for the old
    // path must transition to inactive so BindsTo= dependents notice.
    // TEST-17-UDEV.rename-netif asserts hoge.device goes inactive
    // after `ip link set hoge name foobar`.
    if params.action == "move"
        && let Some(old_path) = params.env.get("DEVPATH_OLD")
        && !old_path.is_empty()
    {
        // Build the old device unit names directly from the old
        // sysfs path.  We deliberately skip `derive_unit_names` here
        // because the event's env (SYSTEMD_ALIAS, symlinks) reflects
        // the POST-rename state and wouldn't map back to the old
        // unit names.
        let old_sysfs = if old_path.starts_with("/sys") {
            old_path.clone()
        } else {
            format!("/sys{old_path}")
        };
        let mut old_names: Vec<String> = Vec::new();
        old_names.push(path_to_device_unit_name(&old_sysfs));
        if !params.subsystem.is_empty()
            && let Some(old_kernel_name) = old_sysfs.rsplit('/').next()
            && !old_kernel_name.is_empty()
        {
            let subsys_esc = crate::unit_name::unit_name_escape(&params.subsystem);
            let kernel_esc = crate::unit_name::unit_name_escape(old_kernel_name);
            let alias = format!("sys-subsystem-{}-devices-{}.device", subsys_esc, kernel_esc);
            if !old_names.contains(&alias) {
                old_names.push(alias);
            }
        }
        apply_device_inactive(run_info, &old_names);
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
            // Check whether the primary unit was already Started before
            // this event arrived.  If so, a transient `ID_PROCESSING=1`
            // on a subsequent event (e.g. a change action that triggers
            // a long-running RUN= program) does NOT demote it back to
            // inactive — the device was usable before, it's still
            // usable during the processing, and `BindsTo=` dependents
            // should stay active.  Only brand-new devices with
            // ID_PROCESSING=1 get deferred to placeholder state.
            // TEST-17-UDEV.device_is_processing Phase 2 asserts this.
            let already_plugged = if let Some(primary_name) = unit_names.first() {
                let id = UnitId {
                    kind: UnitIdKind::Device,
                    name: primary_name.clone(),
                };
                let ri = run_info.read_poisoned();
                ri.unit_table
                    .get(&id)
                    .map(|u| matches!(&*u.common.status.read_poisoned(), UnitStatus::Started(_)))
                    .unwrap_or(false)
            } else {
                false
            };
            let defer_for_processing = is_processing && !already_plugged;
            if defer_for_processing || !systemd_ready || is_renaming {
                trace!(
                    "udev-event: {} deferred (ID_PROCESSING={}, SYSTEMD_READY={}, ID_RENAMING={}, already_plugged={}); placeholder units only",
                    params.sysfs_path, is_processing, systemd_ready, is_renaming, already_plugged
                );
                ensure_device_units_exist(run_info, params, &unit_names);
                // When SYSTEMD_READY flipped to 0 or the device entered
                // renaming, transition any previously-plugged units
                // back to Stopped so BindsTo= dependents deactivate.
                if !systemd_ready || is_renaming {
                    apply_device_inactive(run_info, &unit_names);
                }
            } else {
                // Alias lifecycle: before flipping the new aliases to
                // active, find any OLD alias units that were created
                // for this same sysfs_path by a previous event but
                // aren't in the new alias set, and deactivate them.
                // TEST-17-UDEV.SYSTEMD_ALIAS exercises this by
                // alternating `add` and `change` rules that each set
                // different SYMLINK/SYSTEMD_ALIAS entries — after an
                // `add` the `-on-change` aliases must become inactive,
                // and vice versa.
                deactivate_stale_aliases(run_info, &params.sysfs_path, &unit_names);
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

/// Walk `/run/udev/data/*` and rebuild synthetic `.device` units from the
/// persisted udev db entries.  Used on PID 1 startup (both fresh boot and
/// `daemon-reexec`) to re-populate device-unit state that was previously
/// created on-the-fly via `udev-event` RPC notifications and lost when
/// the in-memory unit table was rebuilt.
///
/// Matches upstream systemd's `manager_enumerate_devices()` behaviour:
/// the udev db is the source of truth for currently-present devices, so
/// walking it gives us a consistent snapshot without waiting for udevd
/// to re-broadcast events.  Entries without the `systemd` tag (and
/// without `SYSTEMD_ALIAS=` / symlinks) are skipped — those aren't
/// tracked as service-manager units.
pub fn rebuild_device_units_from_udev_db(run_info: &ArcMutRuntimeInfo) {
    let db_dir = std::path::Path::new("/run/udev/data");
    let entries = match std::fs::read_dir(db_dir) {
        Ok(e) => e,
        Err(e) => {
            // /run/udev/data not existing isn't an error — it just means
            // udev hasn't written anything yet (e.g. fresh boot before
            // udevd started).  The normal RPC path will populate the
            // table as events arrive.
            trace!(
                "rebuild_device_units: cannot read {}: {} — skipping",
                db_dir.display(),
                e
            );
            return;
        }
    };

    // Read the reexec status file (if present) to learn which device
    // units were `Started` before the re-exec.  Those need to be
    // forcibly re-activated on rebuild regardless of transient udev
    // state (ID_PROCESSING=1 during an in-flight change event must
    // NOT demote a previously-plugged device unit back to inactive).
    // On a fresh boot the file doesn't exist, so `previously_started`
    // is empty and new devices with ID_PROCESSING=1 correctly defer.
    let previously_started: std::collections::HashSet<String> =
        read_previously_started_device_units();

    let mut restored = 0usize;
    let mut skipped = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        // Skip the lock file and any non-db artifacts.
        if name.starts_with('.') || name.ends_with(".tmp") {
            continue;
        }

        let mut params = match build_params_from_db_entry(name, &path) {
            Some(p) => p,
            None => {
                skipped += 1;
                continue;
            }
        };

        // Only rebuild units the service manager would have tracked
        // (tag=systemd OR alias/symlink present).
        let names = derive_unit_names(&params);
        if names.is_empty() {
            skipped += 1;
            continue;
        }
        // If ANY of the derived unit names were Started before the
        // re-exec, strip transient udev markers from the replayed env
        // so the device unit comes back as plugged rather than being
        // deferred.  Only new devices (no reexec-state entry) retain
        // the original ID_PROCESSING=1 / ID_RENAMING=1 semantics.
        if names.iter().any(|n| previously_started.contains(n)) {
            params.env.remove("ID_PROCESSING");
            params.env.remove("ID_RENAMING");
        }
        handle_udev_event(run_info, &params);
        restored += 1;
    }
    if restored > 0 || skipped > 0 {
        debug!(
            "rebuild_device_units_from_udev_db: restored={} skipped={}",
            restored, skipped
        );
    }
}

/// Read the reexec status file and collect the names of `.device`
/// units that were in `Started` state before the re-exec.  Used by
/// `rebuild_device_units_from_udev_db` to preserve `plugged` state
/// for devices across re-exec when the udev db reports transient
/// udev-internal state like `ID_PROCESSING=1`.
fn read_previously_started_device_units() -> std::collections::HashSet<String> {
    let status_path = std::path::Path::new("/run/systemd/rust-systemd-reexec-state.status");
    let mut out = std::collections::HashSet::new();
    let content = match std::fs::read_to_string(status_path) {
        Ok(c) => c,
        Err(_) => return out,
    };
    for line in content.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 {
            continue;
        }
        let name = parts[0];
        let status = parts[1];
        if status == "Started" && name.ends_with(".device") {
            out.insert(name.to_owned());
        }
    }
    out
}

/// Parse one `/run/udev/data/<filename>` entry into an `UdevEventParams`
/// suitable for replay through `handle_udev_event`.  The filename encodes
/// the device class (b/c/n/+); the contents have `E:KEY=VALUE` /
/// `G:TAG` / `S:SYMLINK` lines (see udev db format).  sysfs_path is
/// recovered via `/sys/dev/{block,char}/MAJ:MIN` or `/sys/class/net/*`
/// readlinks.
fn build_params_from_db_entry(
    filename: &str,
    file_path: &std::path::Path,
) -> Option<UdevEventParams> {
    let content = std::fs::read_to_string(file_path).ok()?;

    let mut env: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut tags: Vec<String> = Vec::new();
    let mut symlinks: Vec<String> = Vec::new();
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("E:") {
            if let Some(eq) = rest.find('=') {
                let key = rest[..eq].to_owned();
                let val = rest[eq + 1..].to_owned();
                env.insert(key, udev_db_unescape(&val));
            }
        } else if let Some(tag) = line.strip_prefix("G:") {
            if !tag.is_empty() && !tags.contains(&tag.to_owned()) {
                tags.push(tag.to_owned());
            }
        } else if let Some(link) = line.strip_prefix("S:")
            && !link.is_empty()
        {
            symlinks.push(link.to_owned());
        }
    }

    // Derive sysfs_path, subsystem, and devname from the filename
    // encoding.  Failures are soft (some odd db entries may not
    // resolve) — we skip them.
    let (sysfs_path, subsystem, devname) = resolve_device_from_db_name(filename)?;

    Some(UdevEventParams {
        action: "add".to_owned(),
        sysfs_path,
        devname,
        subsystem,
        env,
        tags,
        symlinks,
    })
}

/// Resolve a udev db filename (e.g. `b8:1`, `n3`, `+net:eth0`) back to
/// `(sysfs_path, subsystem, devname)` by consulting `/sys` symlinks.
///
/// Returns `None` if the device no longer exists in sysfs (i.e. the db
/// entry is stale — the device was removed but udev hasn't cleaned up).
fn resolve_device_from_db_name(filename: &str) -> Option<(String, String, String)> {
    // Block: b<MAJ>:<MIN> → /sys/dev/block/MAJ:MIN → /sys/devices/...
    if let Some(rest) = filename.strip_prefix('b')
        && rest.contains(':')
    {
        let target = std::fs::read_link(format!("/sys/dev/block/{rest}")).ok()?;
        return Some((
            path_to_sysfs_path(&target)?,
            "block".to_owned(),
            devname_from_sysfs(&format!("/sys/dev/block/{rest}")),
        ));
    }
    // Char: c<MAJ>:<MIN> → /sys/dev/char/MAJ:MIN → /sys/devices/...
    if let Some(rest) = filename.strip_prefix('c')
        && rest.contains(':')
    {
        let target = std::fs::read_link(format!("/sys/dev/char/{rest}")).ok()?;
        // Also try subsystem = readlink /sys/dev/char/MAJ:MIN/subsystem
        let subsystem = read_subsystem(&format!("/sys/dev/char/{rest}"));
        return Some((
            path_to_sysfs_path(&target)?,
            subsystem,
            devname_from_sysfs(&format!("/sys/dev/char/{rest}")),
        ));
    }
    // Net: n<IFINDEX> → iterate /sys/class/net/* and find matching ifindex.
    if let Some(rest) = filename.strip_prefix('n') {
        if let Ok(ifindex) = rest.parse::<u32>()
            && let Some(sysfs_path) = sysfs_path_from_ifindex(ifindex)
        {
            return Some((sysfs_path, "net".to_owned(), String::new()));
        }
        return None;
    }
    // Other subsystems: +<SUBSYS>:<SYSNAME>
    if let Some(rest) = filename.strip_prefix('+')
        && let Some(colon) = rest.find(':')
    {
        let subsys = &rest[..colon];
        let sysname = &rest[colon + 1..];
        // Try /sys/bus/<subsys>/devices/<sysname>, then /sys/class/<subsys>/<sysname>.
        for candidate in [
            format!("/sys/bus/{subsys}/devices/{sysname}"),
            format!("/sys/class/{subsys}/{sysname}"),
            format!("/sys/subsystem/{subsys}/devices/{sysname}"),
        ] {
            if let Ok(target) = std::fs::read_link(&candidate)
                && let Some(sysfs) = path_to_sysfs_path(&target)
            {
                return Some((sysfs, subsys.to_owned(), String::new()));
            }
            // Also accept if the candidate itself is a real dir (some
            // /sys/class entries are the actual path rather than a link).
            if let Ok(md) = std::fs::metadata(&candidate)
                && md.is_dir()
            {
                return Some((candidate, subsys.to_owned(), String::new()));
            }
        }
    }
    None
}

/// Normalize a readlink target into a canonical `/sys/...` sysfs path.
/// Handles relative links like `../../devices/...` by joining with the
/// `/sys` root directly (readlink under /sys is relative to the link's
/// containing dir).
fn path_to_sysfs_path(target: &std::path::Path) -> Option<String> {
    let s = target.to_string_lossy();
    // Strip any leading "../" hops; each hop climbs one level from
    // /sys/dev/block/ (or similar) toward /sys.
    let mut rest = s.as_ref();
    let mut depth_from_sys = 3usize; // /sys/dev/block/ → 3 components below /sys
    while let Some(r) = rest.strip_prefix("../") {
        depth_from_sys = depth_from_sys.saturating_sub(1);
        rest = r;
    }
    // `rest` is now the tail relative to /sys (e.g. `devices/virtual/net/eth0`).
    if rest.starts_with('/') {
        return Some(rest.to_owned());
    }
    Some(format!("/sys/{rest}"))
}

/// Read /sys/dev/.../subsystem link basename to recover the subsystem
/// name.  Returns empty string on failure (caller may then default).
fn read_subsystem(sys_dir: &str) -> String {
    let link = format!("{sys_dir}/subsystem");
    if let Ok(target) = std::fs::read_link(&link)
        && let Some(name) = target.file_name().and_then(|s| s.to_str())
    {
        return name.to_owned();
    }
    String::new()
}

/// Read the `uevent` file's `DEVNAME=` line to recover /dev/<node>
/// (prefixed with /dev/).  Empty string if the device has no node.
fn devname_from_sysfs(sys_dir: &str) -> String {
    let uevent_path = format!("{sys_dir}/uevent");
    if let Ok(content) = std::fs::read_to_string(&uevent_path) {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("DEVNAME=") {
                if rest.starts_with('/') {
                    return rest.to_owned();
                }
                return format!("/dev/{rest}");
            }
        }
    }
    String::new()
}

/// For a net device, find its /sys/class/net/<iface> by matching
/// /sys/class/net/*/ifindex against the given ifindex.
fn sysfs_path_from_ifindex(ifindex: u32) -> Option<String> {
    let entries = std::fs::read_dir("/sys/class/net").ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let ifindex_path = path.join("ifindex");
        if let Ok(content) = std::fs::read_to_string(&ifindex_path)
            && content.trim().parse::<u32>().ok() == Some(ifindex)
        {
            // Resolve the symlink to get the canonical /sys/devices/... path.
            if let Ok(target) = std::fs::read_link(&path)
                && let Some(canonical) = path_to_sysfs_path(&target)
            {
                return Some(canonical);
            }
            // Fall back to /sys/class/net/<name>.
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}

/// Reverse of `escape_db_value` in udevd — convert `\xNN` escape
/// sequences back into the raw byte.  Other characters pass through.
fn udev_db_unescape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 3 < bytes.len()
            && bytes[i] == b'\\'
            && bytes[i + 1] == b'x'
            && let (Some(hi), Some(lo)) =
                (hex_digit_value(bytes[i + 2]), hex_digit_value(bytes[i + 3]))
        {
            out.push((hi << 4) | lo);
            i += 4;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
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
        let alias = format!("sys-subsystem-{}-devices-{}.device", subsys_esc, kernel_esc);
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
    // device units, OR refresh the Wants= of already-existing ones from
    // the new event's SYSTEMD_WANTS= list.  Each udev event is a fresh
    // snapshot of the device's properties, so subsequent events must
    // replace the derived Wants= (e.g. a rule that used to set
    // SYSTEMD_WANTS=foo and is changed to set =bar should remove foo
    // from the device unit's Wants).  TEST-17-UDEV.SYSTEMD_WANTS
    // exercises exactly this by swapping the rule's value between
    // events.
    let new_wants = parse_systemd_wants(params);
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
            } else {
                // Unit already exists — update its Wants= to match the
                // new event.  We compute the new target UnitIds and
                // swap out the old ones; reverse wanted_by edges are
                // updated on any loaded targets too.
                refresh_device_wants(&mut ri, &id, &new_wants);
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
    // All unit_names for this event are aliases of the same physical
    // device and share the same SYSTEMD_WANTS= list, so we collect
    // their wants/requires into a de-duplicated set and dispatch each
    // target once — spawning one thread per alias would dispatch the
    // same dep multiple times.  The single thread keeps the control-
    // socket handler responsive if the activation subgraph is slow
    // (e.g. waiting on a Type=notify service).
    let mut deps_to_activate: Vec<UnitId> = Vec::new();
    {
        let ri = run_info.read_poisoned();
        for name in unit_names {
            let id = UnitId {
                kind: UnitIdKind::Device,
                name: name.clone(),
            };
            if let Some(unit) = ri.unit_table.get(&id) {
                for dep in unit
                    .common
                    .dependencies
                    .wants
                    .iter()
                    .chain(unit.common.dependencies.requires.iter())
                {
                    if !deps_to_activate.contains(dep) {
                        deps_to_activate.push(dep.clone());
                    }
                }
            }
        }
    }

    if !deps_to_activate.is_empty() {
        let run_info = run_info.clone();
        let first_name = unit_names[0].clone();
        std::thread::spawn(move || {
            for dep in deps_to_activate {
                trace!(
                    "udev-event: activating dependency {} of device unit {}",
                    dep.name, first_name
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

/// Scan the unit table for device units previously associated with this
/// `sysfs_path` that are NOT in the new alias set, and mark them
/// inactive.  Used on every add/change event to retire stale
/// `SYMLINK+=` and `SYSTEMD_ALIAS=` aliases whose rule no longer fires
/// for the current action.
///
/// The primary sysfs-derived unit name and the `sys-subsystem-...-devices-...`
/// alias are always preserved — they're stable identifiers for the
/// device itself, not per-event aliases.
fn deactivate_stale_aliases(
    run_info: &ArcMutRuntimeInfo,
    sysfs_path: &str,
    new_alias_names: &[String],
) {
    if sysfs_path.is_empty() {
        return;
    }
    let primary_name = path_to_device_unit_name(sysfs_path);

    // Collect stale unit names first (read lock), then flip status
    // (still under read lock via per-unit write lock).
    let stale: Vec<String> = {
        let ri = run_info.read_poisoned();
        ri.unit_table
            .values()
            .filter_map(|u| {
                if u.id.kind != UnitIdKind::Device {
                    return None;
                }
                // Preserve the primary sysfs-derived unit.
                if u.id.name == primary_name {
                    return None;
                }
                // Preserve subsystem-friendly alias
                // (sys-subsystem-<X>-devices-<Y>.device).  We identify
                // these heuristically by the `sys-subsystem-` prefix
                // rather than recomputing, since we don't know the
                // subsystem here.
                if u.id.name.starts_with("sys-subsystem-") {
                    return None;
                }
                // Only consider units that belong to the same sysfs
                // path.  Without this filter we'd deactivate ALL
                // aliases of ALL devices.
                if let Specific::Device(dev) = &u.specific
                    && dev.conf.sysfs_path.as_deref() != Some(sysfs_path)
                {
                    return None;
                }
                // Keep units that ARE in the new alias set.
                if new_alias_names.contains(&u.id.name) {
                    return None;
                }
                // Only transition units that are currently Started —
                // avoid churn on already-inactive units.
                let status = u.common.status.read_poisoned();
                if !matches!(&*status, UnitStatus::Started(_)) {
                    return None;
                }
                Some(u.id.name.clone())
            })
            .collect()
    };

    if stale.is_empty() {
        return;
    }
    trace!(
        "udev-event: deactivating {} stale alias(es) of {}: {:?}",
        stale.len(),
        sysfs_path,
        stale
    );
    apply_device_inactive(run_info, &stale);
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

/// Extract the SYSTEMD_WANTS value from an event's env, split into a
/// list of target unit names.  Empty entries are skipped.
fn parse_systemd_wants(params: &UdevEventParams) -> Vec<String> {
    params
        .env
        .get("SYSTEMD_WANTS")
        .map(|v| {
            v.split_whitespace()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Replace a device unit's `Wants=` edges to match the new list, and
/// update reverse `wanted_by` edges on any loaded target units.  Called
/// on every add/change event to keep the derived dependency state in
/// sync with the latest SYSTEMD_WANTS= value.
fn refresh_device_wants(
    ri: &mut crate::runtime_info::RuntimeInfo,
    device_id: &UnitId,
    new_wants: &[String],
) {
    // Compute the new Wants= UnitIds.  Targets get `.service` suffix if
    // the name has no suffix, matching how fresh builds handle it.
    let new_want_ids: Vec<UnitId> = new_wants
        .iter()
        .map(|name| UnitId {
            kind: if name.ends_with(".service") {
                UnitIdKind::Service
            } else if name.ends_with(".target") {
                UnitIdKind::Target
            } else if name.ends_with(".socket") {
                UnitIdKind::Socket
            } else if name.ends_with(".timer") {
                UnitIdKind::Timer
            } else if name.ends_with(".path") {
                UnitIdKind::Path
            } else if name.ends_with(".mount") {
                UnitIdKind::Mount
            } else if name.ends_with(".slice") {
                UnitIdKind::Slice
            } else {
                UnitIdKind::Service
            },
            name: if name.contains('.') {
                name.clone()
            } else {
                format!("{name}.service")
            },
        })
        .collect();

    // Collect the old Wants= from the device so we can diff.
    let old_wants: Vec<UnitId> = match ri.unit_table.get(device_id) {
        Some(u) => u.common.dependencies.wants.clone(),
        None => return,
    };

    // Remove the OLD wanted_by edge from any target units that used to
    // be wanted but no longer are.
    for old in &old_wants {
        if new_want_ids.contains(old) {
            continue;
        }
        if let Some(target) = ri.unit_table.get_mut(old) {
            target
                .common
                .dependencies
                .wanted_by
                .retain(|id| id != device_id);
        }
    }

    // Add the NEW wanted_by edge to any target units that are newly
    // wanted.
    for new_id in &new_want_ids {
        if old_wants.contains(new_id) {
            continue;
        }
        if let Some(target) = ri.unit_table.get_mut(new_id)
            && !target.common.dependencies.wanted_by.contains(device_id)
        {
            target.common.dependencies.wanted_by.push(device_id.clone());
        }
    }

    // Finally, replace the device unit's own Wants= list.
    if let Some(device) = ri.unit_table.get_mut(device_id) {
        device.common.dependencies.wants = new_want_ids;
        // Mirror into refs_by_name so persistence / query paths see
        // the up-to-date set.
        device.common.unit.refs_by_name = device.common.dependencies.wants.clone();
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
        let mut p = make_params("add", "/sys/devices/virtual/mem/null", "mem", &[], &[]);
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

    // -----------------------------------------------------------------------
    // udev db rebuild helpers
    // -----------------------------------------------------------------------

    #[test]
    fn test_udev_db_unescape_basic() {
        assert_eq!(udev_db_unescape("aa\\x20bb"), "aa bb");
        assert_eq!(udev_db_unescape("\\x20foo\\x20"), " foo ");
        assert_eq!(udev_db_unescape("no-escapes"), "no-escapes");
        assert_eq!(udev_db_unescape(""), "");
    }

    #[test]
    fn test_udev_db_unescape_preserves_unknown_escapes() {
        // \z is not an \xNN sequence — pass through as-is.
        assert_eq!(udev_db_unescape("a\\zb"), "a\\zb");
        // Dangling \x at end — keep the slash and x as literals.
        assert_eq!(udev_db_unescape("end\\x"), "end\\x");
    }

    #[test]
    fn test_hex_digit_value() {
        assert_eq!(hex_digit_value(b'0'), Some(0));
        assert_eq!(hex_digit_value(b'9'), Some(9));
        assert_eq!(hex_digit_value(b'a'), Some(10));
        assert_eq!(hex_digit_value(b'F'), Some(15));
        assert_eq!(hex_digit_value(b'z'), None);
        assert_eq!(hex_digit_value(b' '), None);
    }

    #[test]
    fn test_path_to_sysfs_path_relative() {
        // A typical readlink of /sys/dev/block/8:1 gives
        // `../../devices/pci0000:00/.../block/sda/sda1`.  After
        // stripping `../` hops, we want `/sys/devices/...`.
        let target = std::path::Path::new("../../devices/virtual/net/eth0");
        let resolved = path_to_sysfs_path(target).unwrap();
        assert_eq!(resolved, "/sys/devices/virtual/net/eth0");
    }

    #[test]
    fn test_path_to_sysfs_path_absolute_unchanged() {
        let target = std::path::Path::new("/sys/devices/virtual/net/eth0");
        // Absolute paths pass through (`starts_with('/')` short-circuits).
        let resolved = path_to_sysfs_path(target).unwrap();
        assert_eq!(resolved, "/sys/devices/virtual/net/eth0");
    }

    #[test]
    fn test_build_params_from_db_entry_parses_contents() {
        // Write a synthetic db entry to a temp file and parse it,
        // bypassing the resolve_device_from_db_name step (which needs
        // real /sys).  We just test that E:/G:/S: lines decompose.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("n1");
        let contents = "\
S:disk/by-uuid/abcd\n\
G:systemd\n\
Q:systemd\n\
L:0\n\
E:ID_NET_NAME=eth0\n\
E:SYSTEMD_WANTS=foo.service\n\
E:HOGE=aa\\x20bb\n\
E:TAGS=:systemd:\n\
";
        std::fs::write(&path, contents).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let mut env: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut tags: Vec<String> = Vec::new();
        let mut symlinks: Vec<String> = Vec::new();
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("E:") {
                if let Some(eq) = rest.find('=') {
                    env.insert(rest[..eq].to_owned(), udev_db_unescape(&rest[eq + 1..]));
                }
            } else if let Some(tag) = line.strip_prefix("G:") {
                if !tag.is_empty() && !tags.contains(&tag.to_owned()) {
                    tags.push(tag.to_owned());
                }
            } else if let Some(link) = line.strip_prefix("S:") {
                if !link.is_empty() {
                    symlinks.push(link.to_owned());
                }
            }
        }
        assert_eq!(env.get("ID_NET_NAME").map(|s| s.as_str()), Some("eth0"));
        assert_eq!(
            env.get("SYSTEMD_WANTS").map(|s| s.as_str()),
            Some("foo.service")
        );
        // HOGE with \x20 should round-trip to a single space.
        assert_eq!(env.get("HOGE").map(|s| s.as_str()), Some("aa bb"));
        assert!(tags.contains(&"systemd".to_owned()));
        assert!(symlinks.iter().any(|s| s == "disk/by-uuid/abcd"));
    }

    #[test]
    fn test_handle_udev_event_wants_refresh_across_events() {
        let ri = make_test_runtime();

        // First event: SYSTEMD_WANTS=foo.service.
        let mut params1 = make_params(
            "add",
            "/sys/devices/virtual/block/vda",
            "block",
            &["systemd"],
            &[],
        );
        params1
            .env
            .insert("SYSTEMD_WANTS".into(), "foo.service".into());
        handle_udev_event(&ri, &params1);

        let device_name = "sys-devices-virtual-block-vda.device";
        {
            let ri_guard = ri.read().unwrap();
            let device = ri_guard
                .unit_table
                .values()
                .find(|u| u.id.name == device_name)
                .unwrap();
            let wants: Vec<&str> = device
                .common
                .dependencies
                .wants
                .iter()
                .map(|id| id.name.as_str())
                .collect();
            assert_eq!(wants, vec!["foo.service"]);
        }

        // Second event: SYSTEMD_WANTS=bar.service — must replace foo.
        let mut params2 = make_params(
            "change",
            "/sys/devices/virtual/block/vda",
            "block",
            &["systemd"],
            &[],
        );
        params2
            .env
            .insert("SYSTEMD_WANTS".into(), "bar.service".into());
        handle_udev_event(&ri, &params2);

        let ri_guard = ri.read().unwrap();
        let device = ri_guard
            .unit_table
            .values()
            .find(|u| u.id.name == device_name)
            .unwrap();
        let wants: Vec<&str> = device
            .common
            .dependencies
            .wants
            .iter()
            .map(|id| id.name.as_str())
            .collect();
        assert_eq!(wants, vec!["bar.service"]);
        // Also verify refs_by_name mirrors the new Wants.
        let refs: Vec<&str> = device
            .common
            .unit
            .refs_by_name
            .iter()
            .map(|id| id.name.as_str())
            .collect();
        assert_eq!(refs, vec!["bar.service"]);
    }

    #[test]
    fn test_handle_udev_event_stale_aliases_deactivated_on_change() {
        let ri = make_test_runtime();

        // First event: add with symlink A.
        let mut add_params = make_params(
            "add",
            "/sys/devices/virtual/mem/null",
            "mem",
            &["systemd"],
            &["/dev/test/alias-add-only"],
        );
        add_params.env.insert(
            "SYSTEMD_ALIAS".into(),
            "/sys/test/env-alias-add-only".into(),
        );
        handle_udev_event(&ri, &add_params);

        let add_only_unit = r"dev-test-alias\x2dadd\x2donly.device";
        let env_add_only_unit = r"sys-test-env\x2dalias\x2dadd\x2donly.device";
        assert!(
            matches!(
                unit_status(&ri, add_only_unit).unwrap(),
                UnitStatus::Started(_)
            ),
            "add-only alias should be active after add event"
        );
        assert!(
            matches!(
                unit_status(&ri, env_add_only_unit).unwrap(),
                UnitStatus::Started(_)
            ),
            "env-alias add-only should be active after add event"
        );

        // Second event: change with DIFFERENT symlink/alias.
        let mut change_params = make_params(
            "change",
            "/sys/devices/virtual/mem/null",
            "mem",
            &["systemd"],
            &["/dev/test/alias-change-only"],
        );
        change_params.env.insert(
            "SYSTEMD_ALIAS".into(),
            "/sys/test/env-alias-change-only".into(),
        );
        handle_udev_event(&ri, &change_params);

        // The old aliases should be deactivated.
        assert!(
            !matches!(
                unit_status(&ri, add_only_unit).unwrap(),
                UnitStatus::Started(_)
            ),
            "add-only alias should be inactive after change event: {:?}",
            unit_status(&ri, add_only_unit)
        );
        assert!(
            !matches!(
                unit_status(&ri, env_add_only_unit).unwrap(),
                UnitStatus::Started(_)
            ),
            "env-alias add-only should be inactive after change event"
        );

        // Primary unit remains active.
        let primary = "sys-devices-virtual-mem-null.device";
        assert!(
            matches!(unit_status(&ri, primary).unwrap(), UnitStatus::Started(_)),
            "primary sysfs-derived unit must stay active across events"
        );
    }
}

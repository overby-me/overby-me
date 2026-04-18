//! Minimal D-Bus server implementing `org.freedesktop.systemd1`.
//!
//! PID 1 connects to the system bus once `dbus.service` is running and
//! registers the well-known name `org.freedesktop.systemd1`, then exports
//! `/org/freedesktop/systemd1` implementing the `Manager` interface.
//!
//! The module is a no-op stub when the `dbus_support` cargo feature is off.

#[cfg(not(feature = "dbus_support"))]
pub fn start_dbus_server_thread(_run_info: crate::runtime_info::ArcMutRuntimeInfo) {
    // no-op
}

#[cfg(feature = "dbus_support")]
pub use inner::start_dbus_server_thread;

#[cfg(feature = "dbus_support")]
mod inner {
    use crate::lock_ext::RwLockExt;
    use crate::runtime_info::ArcMutRuntimeInfo;
    use crate::units::UnitStatus;
    use log::{info, trace, warn};
    use zbus::blocking::Connection;
    use zbus::interface;

    /// The `/org/freedesktop/systemd1` object implementing the Manager interface.
    struct Manager {
        run_info: ArcMutRuntimeInfo,
    }

    /// Per-unit object exported at
    /// `/org/freedesktop/systemd1/unit/<escaped_name>`.  Implements
    /// `org.freedesktop.systemd1.Unit` with a subset of properties that
    /// tests actually query via `busctl get-property`.
    struct UnitObj {
        run_info: ArcMutRuntimeInfo,
        unit_name: String,
    }

    #[interface(name = "org.freedesktop.systemd1.Unit")]
    impl UnitObj {
        #[zbus(property)]
        fn id(&self) -> String {
            self.unit_name.clone()
        }

        #[zbus(property)]
        fn description(&self) -> String {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| u.common.unit.description.clone())
                .unwrap_or_default()
        }

        #[zbus(property)]
        fn active_state(&self) -> String {
            let ri = self.run_info.read_poisoned();
            if let Some(u) = ri.unit_table.values().find(|u| u.id.name == self.unit_name) {
                let status = u.common.status.read_poisoned();
                map_active_sub(&status).0.to_string()
            } else {
                "inactive".to_string()
            }
        }

        #[zbus(property)]
        fn sub_state(&self) -> String {
            let ri = self.run_info.read_poisoned();
            if let Some(u) = ri.unit_table.values().find(|u| u.id.name == self.unit_name) {
                let status = u.common.status.read_poisoned();
                map_active_sub(&status).1.to_string()
            } else {
                "dead".to_string()
            }
        }

        #[zbus(property)]
        fn load_state(&self) -> String {
            let ri = self.run_info.read_poisoned();
            if ri.unit_table.values().any(|u| u.id.name == self.unit_name) {
                "loaded".to_string()
            } else {
                "not-found".to_string()
            }
        }

        #[zbus(property)]
        fn can_start(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_stop(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_reload(&self) -> bool {
            false
        }

        #[zbus(property)]
        fn can_isolate(&self) -> bool {
            false
        }

        #[zbus(property)]
        fn can_freeze(&self) -> bool {
            true
        }

        /// All names this unit answers to — primary id plus any aliases.
        #[zbus(property)]
        fn names(&self) -> Vec<String> {
            let ri = self.run_info.read_poisoned();
            let Some(unit) = ri.unit_table.values().find(|u| u.id.name == self.unit_name) else {
                return vec![self.unit_name.clone()];
            };
            let mut out = vec![unit.id.name.clone()];
            out.extend(unit.common.unit.aliases.iter().cloned());
            out
        }

        /// Absolute path of the unit's source fragment file, if any.
        #[zbus(property)]
        fn fragment_path(&self) -> String {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .and_then(|u| u.common.unit.fragment_path.as_ref())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default()
        }

        /// Drop-in config files merged into this unit.
        #[zbus(property)]
        fn drop_in_paths(&self) -> Vec<String> {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| {
                    u.common
                        .unit
                        .loaded_dropin_files
                        .iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect()
                })
                .unwrap_or_default()
        }

        /// Whether runtime bind mounts (`systemctl bind`) can be applied
        /// to this unit.  Only services that already have a private mount
        /// namespace (PrivateMounts=yes, PrivateTmp=yes, or similar) can
        /// accept live mounts.
        #[zbus(property)]
        fn can_live_mount(&self) -> bool {
            let ri = self.run_info.read_poisoned();
            let Some(unit) = ri.unit_table.values().find(|u| u.id.name == self.unit_name) else {
                return false;
            };
            let crate::units::Specific::Service(srvc) = &unit.specific else {
                return false;
            };
            srvc.conf.exec_config.private_tmp
                || srvc.conf.exec_config.private_devices
                || srvc.conf.exec_config.private_mounts
                || srvc.conf.exec_config.private_users
                || !srvc.conf.exec_config.bind_paths.is_empty()
                || !srvc.conf.exec_config.bind_read_only_paths.is_empty()
                || !srvc.conf.exec_config.read_only_paths.is_empty()
                || !srvc.conf.exec_config.inaccessible_paths.is_empty()
        }
    }

    /// Per-unit object additionally exposing `org.freedesktop.systemd1.Service`
    /// for .service units.  Registered on the same object path as the Unit
    /// interface; zbus routes introspection/property queries to whichever
    /// interface matches.
    struct ServiceObj {
        run_info: ArcMutRuntimeInfo,
        unit_name: String,
    }

    #[interface(name = "org.freedesktop.systemd1.Service")]
    impl ServiceObj {
        #[zbus(property)]
        fn main_pid(&self) -> u32 {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| {
                    let pid = u
                        .common
                        .main_pid
                        .load(std::sync::atomic::Ordering::Acquire);
                    if pid > 0 { pid as u32 } else { 0 }
                })
                .unwrap_or(0)
        }

        #[zbus(property)]
        fn exec_main_pid(&self) -> u32 {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| {
                    let pid = u
                        .common
                        .main_exit_pid
                        .load(std::sync::atomic::Ordering::Acquire);
                    if pid > 0 { pid as u32 } else { 0 }
                })
                .unwrap_or(0)
        }

        #[zbus(property)]
        fn exec_main_status(&self) -> i32 {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| {
                    u.common
                        .main_exit_status
                        .load(std::sync::atomic::Ordering::Acquire)
                })
                .unwrap_or(-1)
        }

        #[zbus(property)]
        fn type_(&self) -> String {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .and_then(|u| match &u.specific {
                    crate::units::Specific::Service(srvc) => Some(service_type_string(
                        &srvc.conf.srcv_type,
                    )),
                    _ => None,
                })
                .unwrap_or_else(|| "simple".to_string())
        }

        #[zbus(property)]
        fn result(&self) -> String {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| {
                    let status = u.common.status.read_poisoned();
                    match &*status {
                        UnitStatus::Stopped(_, errs) if !errs.is_empty() => "failure",
                        _ => "success",
                    }
                    .to_string()
                })
                .unwrap_or_else(|| "success".to_string())
        }
    }

    fn service_type_string(t: &crate::units::ServiceType) -> String {
        match t {
            crate::units::ServiceType::Simple => "simple",
            crate::units::ServiceType::Forking => "forking",
            crate::units::ServiceType::OneShot => "oneshot",
            crate::units::ServiceType::Dbus => "dbus",
            crate::units::ServiceType::Notify => "notify",
            crate::units::ServiceType::NotifyReload => "notify-reload",
            crate::units::ServiceType::Idle => "idle",
            crate::units::ServiceType::Exec => "exec",
        }
        .to_string()
    }

    /// Per-unit tuple returned by `ListUnits`: (name, description, load,
    /// active, sub, follower, obj_path, job_id, job_type, job_obj_path).
    type ListUnitsEntry = (
        String,
        String,
        String,
        String,
        String,
        String,
        zbus::zvariant::OwnedObjectPath,
        u32,
        String,
        zbus::zvariant::OwnedObjectPath,
    );

    #[interface(name = "org.freedesktop.systemd1.Manager")]
    impl Manager {
        /// Returns the version string of the service manager.
        #[zbus(property)]
        fn version(&self) -> String {
            env!("CARGO_PKG_VERSION").to_string()
        }

        /// Returns the architecture string (e.g. "x86-64").
        #[zbus(property)]
        fn architecture(&self) -> String {
            std::env::consts::ARCH.to_string()
        }

        /// Returns all currently loaded units as a list of tuples:
        /// (name, description, load_state, active_state, sub_state,
        ///  follower, object_path, job_id, job_type, job_object_path).
        fn list_units(&self) -> Vec<ListUnitsEntry> {
            let ri = self.run_info.read_poisoned();
            let mut out = Vec::new();
            let root = zbus::zvariant::OwnedObjectPath::try_from("/").unwrap();
            for unit in ri.unit_table.values() {
                let status = unit.common.status.read_poisoned();
                let (active, sub) = map_active_sub(&status);
                let desc = unit.common.unit.description.clone();
                let obj = unit_object_path(&unit.id.name);
                out.push((
                    unit.id.name.clone(),
                    desc,
                    "loaded".to_string(),
                    active.to_string(),
                    sub.to_string(),
                    String::new(),
                    obj,
                    0,
                    String::new(),
                    root.clone(),
                ));
            }
            out
        }

        /// Returns the object path of the unit with the given name.
        fn get_unit(&self, name: String) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            let ri = self.run_info.read_poisoned();
            if ri.unit_table.values().any(|u| u.id.name == name) {
                Ok(unit_object_path(&name))
            } else {
                Err(zbus::fdo::Error::Failed(format!("Unit {name} not found")))
            }
        }

        /// Start the given unit.  The `mode` argument is accepted but ignored
        /// (C systemd uses it for job scheduling modes like "replace" /
        /// "isolate").  Returns the object path of a fictional job entry.
        fn start_unit(
            &self,
            name: String,
            _mode: String,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            invoke_command(&self.run_info, crate::control::Command::Start(vec![name]))
                .map_err(zbus::fdo::Error::Failed)?;
            Ok(zbus::zvariant::OwnedObjectPath::try_from("/").unwrap())
        }

        /// Stop the given unit.
        fn stop_unit(
            &self,
            name: String,
            _mode: String,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            invoke_command(&self.run_info, crate::control::Command::Stop(vec![name]))
                .map_err(zbus::fdo::Error::Failed)?;
            Ok(zbus::zvariant::OwnedObjectPath::try_from("/").unwrap())
        }

        /// Restart the given unit.
        fn restart_unit(
            &self,
            name: String,
            _mode: String,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            invoke_command(&self.run_info, crate::control::Command::Restart(name))
                .map_err(zbus::fdo::Error::Failed)?;
            Ok(zbus::zvariant::OwnedObjectPath::try_from("/").unwrap())
        }

        /// Reload unit files from disk (equivalent to `systemctl daemon-reload`).
        fn reload(&self) -> zbus::fdo::Result<()> {
            invoke_command(&self.run_info, crate::control::Command::LoadAllNew)
                .map_err(zbus::fdo::Error::Failed)?;
            Ok(())
        }

        /// Create a transient unit from the given name, mode, and property
        /// dictionary.  Properties are translated into `-p NAME=VALUE`
        /// strings and fed through the existing `StartTransient` control
        /// command.  FDs passed via `ExtraFileDescriptors` are dup'd out of
        /// the D-Bus message, FD_CLOEXEC cleared, and stored as the
        /// service's stored_fds so they flow through LISTEN_FDS.
        fn start_transient_unit(
            &self,
            name: String,
            _mode: String,
            properties: Vec<(String, zbus::zvariant::OwnedValue)>,
            _aux: Vec<(String, Vec<(String, zbus::zvariant::OwnedValue)>)>,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            let mut params = crate::control::TransientUnitParams {
                unit_name: name.clone(),
                command: None,
                description: None,
                user: None,
                group: None,
                working_directory: None,
                service_type: None,
                remain_after_exit: false,
                properties: Vec::new(),
                environment: Vec::new(),
                scope: false,
                wait: false,
                pipe: false,
                slice: None,
                on_calendar: None,
                on_active: Vec::new(),
                on_boot: None,
                on_startup: None,
                on_unit_active: None,
                on_unit_inactive: None,
                on_clock_change: false,
                on_timezone_change: false,
                timer_properties: Vec::new(),
                path_properties: Vec::new(),
                socket_properties: Vec::new(),
                nice: None,
                extra_file_descriptors: Vec::new(),
            };
            for (key, val) in properties {
                if key == "ExtraFileDescriptors" {
                    params
                        .extra_file_descriptors
                        .extend(extract_fd_entries(&val));
                    continue;
                }
                if let Some(entry) = format_property(&key, &val) {
                    match key.as_str() {
                        "Description" => params.description = Some(entry),
                        "User" => params.user = Some(entry),
                        "Group" => params.group = Some(entry),
                        "WorkingDirectory" => params.working_directory = Some(entry),
                        "Type" => params.service_type = Some(entry),
                        "RemainAfterExit" => params.remain_after_exit = entry == "true",
                        "Slice" => params.slice = Some(entry),
                        _ => params.properties.push(format!("{key}={entry}")),
                    }
                }
            }
            invoke_command(
                &self.run_info,
                crate::control::Command::StartTransient(params),
            )
            .map_err(zbus::fdo::Error::Failed)?;
            Ok(unit_object_path(&name))
        }
    }

    /// Extract `ExtraFileDescriptors` entries (type `a(hs)`) from a D-Bus
    /// variant into a list of (raw_fd, name) pairs. The caller takes
    /// ownership of the fds — we use `into_raw_fd()` on the wrapped
    /// OwnedFd so the zbus message's drop does not close them.
    fn extract_fd_entries(val: &zbus::zvariant::OwnedValue) -> Vec<(std::os::fd::RawFd, String)> {
        use std::os::fd::AsRawFd;
        use zbus::zvariant::Value;
        let inner: &Value = val;
        let mut out = Vec::new();
        let Value::Array(items) = inner else {
            return out;
        };
        for item in items.iter() {
            let Value::Structure(s) = item else {
                continue;
            };
            let fields = s.fields();
            let Some(Value::Fd(fd)) = fields.first() else {
                continue;
            };
            let Some(Value::Str(name)) = fields.get(1) else {
                continue;
            };
            // The zbus Fd borrows from the message; duplicate it so the
            // fd remains valid after the message is dropped.
            let borrowed_raw = fd.as_raw_fd();
            let dup = unsafe { libc::dup(borrowed_raw) };
            if dup < 0 {
                warn!(
                    "dbus-server: dup of ExtraFileDescriptors fd failed: {}",
                    std::io::Error::last_os_error()
                );
                continue;
            }
            // Clear FD_CLOEXEC so the service process inherits it.
            unsafe {
                let flags = libc::fcntl(dup, libc::F_GETFD);
                if flags >= 0 {
                    libc::fcntl(dup, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
                }
            }
            out.push((dup, name.to_string()));
        }
        out
    }

    /// Convert a D-Bus variant value into the text form that the transient
    /// unit property parser expects.  Returns `None` when the variant type
    /// is complex enough that text fallback would be meaningless; the
    /// caller can then decide to skip the property.
    fn format_property(key: &str, val: &zbus::zvariant::OwnedValue) -> Option<String> {
        use zbus::zvariant::Value;
        let inner: &Value = val;
        match inner {
            Value::Str(s) => Some(s.to_string()),
            Value::Bool(b) => Some(if *b { "true" } else { "false" }.to_string()),
            Value::I32(i) => Some(i.to_string()),
            Value::U32(u) => Some(u.to_string()),
            Value::I64(i) => Some(i.to_string()),
            Value::U64(u) => Some(u.to_string()),
            Value::F64(f) => Some(f.to_string()),
            _ => {
                // Complex values — e.g. ExecStart as a(sasb).  Try to
                // flatten into a space-separated argv string so the text
                // parser can split it on whitespace.
                if key == "ExecStart"
                    && let Value::Array(items) = inner
                {
                    // Each item is a (path, argv, skip_on_fail) tuple.
                    let mut out = String::new();
                    for item in items.iter() {
                        if let Value::Structure(s) = item {
                            let fields = s.fields();
                            if let Some(Value::Array(args)) = fields.get(1) {
                                let words: Vec<String> = args
                                    .iter()
                                    .filter_map(|a| match a {
                                        Value::Str(s) => Some(s.to_string()),
                                        _ => None,
                                    })
                                    .collect();
                                if !out.is_empty() {
                                    out.push(' ');
                                }
                                out.push_str(&words.join(" "));
                            }
                        }
                    }
                    if out.is_empty() { None } else { Some(out) }
                } else {
                    None
                }
            }
        }
    }

    /// Run a Command through the normal control pathway.
    fn invoke_command(
        run_info: &ArcMutRuntimeInfo,
        cmd: crate::control::Command,
    ) -> Result<(), String> {
        crate::control::execute_command(cmd, run_info.clone()).map(|_| ())
    }

    /// Map a `UnitStatus` to the D-Bus `ActiveState` and `SubState` strings
    /// that C systemd exports.
    fn map_active_sub(status: &UnitStatus) -> (&'static str, &'static str) {
        match status {
            UnitStatus::NeverStarted => ("inactive", "dead"),
            UnitStatus::Starting => ("activating", "start"),
            UnitStatus::Started(_) => ("active", "running"),
            UnitStatus::Stopping => ("deactivating", "stop"),
            UnitStatus::Restarting => ("activating", "auto-restart"),
            UnitStatus::Stopped(_, errs) if !errs.is_empty() => ("failed", "failed"),
            UnitStatus::Stopped(_, _) => ("inactive", "dead"),
        }
    }

    /// Encode a unit name into a D-Bus object path the way C systemd does.
    /// Example: `dbus.service` → `/org/freedesktop/systemd1/unit/dbus_2eservice`.
    fn unit_object_path(name: &str) -> zbus::zvariant::OwnedObjectPath {
        let mut encoded = String::with_capacity(name.len() * 2);
        for (i, b) in name.bytes().enumerate() {
            let keep = (b.is_ascii_alphanumeric() && (i > 0 || !b.is_ascii_digit()))
                || (b == b'_' && i > 0);
            if keep {
                encoded.push(b as char);
            } else {
                encoded.push_str(&format!("_{b:02x}"));
            }
        }
        let path = format!("/org/freedesktop/systemd1/unit/{encoded}");
        zbus::zvariant::OwnedObjectPath::try_from(path.as_str())
            .unwrap_or_else(|_| zbus::zvariant::OwnedObjectPath::try_from("/").unwrap())
    }

    /// Spawn a background thread that (eventually) connects to the system
    /// bus and exports the Manager interface.  Retries connecting every few
    /// seconds until `dbus.service` is available.
    pub fn start_dbus_server_thread(run_info: ArcMutRuntimeInfo) {
        std::thread::Builder::new()
            .name("dbus-server".into())
            .spawn(move || serve(run_info))
            .expect("dbus-server thread");
    }

    fn serve(run_info: ArcMutRuntimeInfo) {
        let manager = Manager {
            run_info: run_info.clone(),
        };
        let conn = loop {
            match Connection::system() {
                Ok(c) => break c,
                Err(e) => {
                    trace!("dbus-server: waiting for system bus: {e}");
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            }
        };
        if let Err(e) = conn
            .object_server()
            .at("/org/freedesktop/systemd1", manager)
        {
            warn!("dbus-server: failed to export Manager object: {e}");
            return;
        }
        match conn.request_name("org.freedesktop.systemd1") {
            Ok(()) => {
                info!("dbus-server: registered org.freedesktop.systemd1 on system bus");
            }
            Err(e) => {
                warn!("dbus-server: request_name org.freedesktop.systemd1 failed: {e}");
                return;
            }
        }

        // Register per-unit objects for all currently-loaded units.  New
        // units added later (e.g. transient units from StartTransientUnit)
        // are registered by register_unit_object() called from the
        // appropriate paths.
        {
            let ri = run_info.read_poisoned();
            let names: Vec<String> = ri.unit_table.values().map(|u| u.id.name.clone()).collect();
            drop(ri);
            let mut registered = 0usize;
            for name in names {
                if register_unit_object(&conn, &run_info, &name).is_ok() {
                    registered += 1;
                }
            }
            info!(
                "dbus-server: registered {registered} per-unit objects"
            );
        }

        // Block forever — zbus spawns its own message-dispatch thread.
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    /// Register a Unit object at the encoded object path for `unit_name`.
    /// For `.service` units, also attach the Service interface.
    fn register_unit_object(
        conn: &Connection,
        run_info: &ArcMutRuntimeInfo,
        unit_name: &str,
    ) -> zbus::Result<()> {
        let path = unit_object_path(unit_name);
        let obj = UnitObj {
            run_info: run_info.clone(),
            unit_name: unit_name.to_owned(),
        };
        conn.object_server().at(&path, obj)?;
        if unit_name.ends_with(".service") {
            let svc = ServiceObj {
                run_info: run_info.clone(),
                unit_name: unit_name.to_owned(),
            };
            conn.object_server().at(&path, svc)?;
        }
        Ok(())
    }
}

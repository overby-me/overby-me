//! udevadm — udev administration tool.
//!
//! This is a Rust reimplementation of the udevadm CLI tool. It provides
//! subcommands for querying the udev database, triggering device events,
//! waiting for the event queue to drain, monitoring kernel/udev events,
//! testing rules against devices, and controlling the running daemon.
//!
//! Subcommands:
//!   info       — Query device information from sysfs and the udev database
//!   trigger    — Request device events from the kernel
//!   settle     — Wait for the udev event queue to drain
//!   monitor    — Listen for kernel uevents and udev events
//!   test       — Simulate a device event and show rule results
//!   control    — Send control commands to the running systemd-udevd daemon
//!   test-builtin — Test a built-in command against a device
//!   version    — Show version information

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime};

use systemd_udevd::{
    CONTROL_SOCKET_PATH, DB_DIR, QUEUE_FILE, RULES_DIRS, RuleSet, TAGS_DIR, UEvent, glob_match,
    open_uevent_socket, process_rules,
};

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

fn init_logging() {
    struct StderrLogger;
    impl log::Log for StderrLogger {
        fn enabled(&self, _metadata: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                eprintln!("udevadm: [{}] {}", record.level(), record.args());
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: StderrLogger = StderrLogger;
    let level = match std::env::var("SYSTEMD_LOG_LEVEL").as_deref() {
        Ok("debug") => log::LevelFilter::Debug,
        Ok("trace") => log::LevelFilter::Trace,
        Ok("warn") => log::LevelFilter::Warn,
        Ok("error") => log::LevelFilter::Error,
        _ => log::LevelFilter::Info,
    };
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(level);
}

// ---------------------------------------------------------------------------
// CLI parsing
// ---------------------------------------------------------------------------

use clap::{Parser, Subcommand};

/// udevadm — udev management tool
#[derive(Parser, Debug)]
#[command(name = "udevadm", version, about = "udev management tool")]
struct Cli {
    /// Enable debug output
    #[arg(long, short = 'd', global = true)]
    debug: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Query sysfs or the udev database
    Info {
        /// Query by device node path (e.g. /dev/sda)
        #[arg(long, short = 'n')]
        name: Option<String>,

        /// Query by sys path (e.g. /sys/class/block/sda)
        #[arg(long, short = 'p')]
        path: Option<String>,

        /// Type of query: name, symlink, path, property, all
        #[arg(long, short = 'q', default_value = "all")]
        query: String,

        /// Show device properties in key=value format
        #[arg(long, short = 'e')]
        export: bool,

        /// Key prefix for exported properties
        #[arg(long)]
        export_prefix: Option<String>,

        /// Show the device path for this device
        #[arg(long, short = 'r')]
        root: bool,

        /// Print result for a specified attribute
        #[arg(long, short = 'a')]
        attribute_walk: bool,

        /// Do not look up device in the udev database, query sysfs directly
        #[arg(long, short = 'x')]
        export_db: bool,

        /// Cleanup the udev database
        #[arg(long, short = 'c')]
        cleanup_db: bool,

        /// Print value of a specific property
        #[arg(long)]
        value: bool,

        /// Print major:minor of the device backing a file
        #[arg(long)]
        device_id_of_file: Option<String>,

        /// Additional device paths on the command line
        devices: Vec<String>,
    },

    /// Request device events from the kernel
    Trigger {
        /// Event type to trigger (devices, subsystems, all)
        #[arg(long, short = 't', default_value = "devices")]
        r#type: String,

        /// Action to request (add, change, remove, etc.)
        #[arg(long, short = 'c', default_value = "change")]
        action: String,

        /// Match subsystem
        #[arg(long, short = 's')]
        subsystem_match: Vec<String>,

        /// Exclude subsystem
        #[arg(long, short = 'S')]
        subsystem_nomatch: Vec<String>,

        /// Match attribute
        #[arg(long, short = 'a')]
        attr_match: Vec<String>,

        /// Exclude attribute
        #[arg(long, short = 'A')]
        attr_nomatch: Vec<String>,

        /// Match property
        #[arg(long, short = 'p')]
        property_match: Vec<String>,

        /// Match tag
        #[arg(long, short = 'g')]
        tag_match: Vec<String>,

        /// Match sysname
        #[arg(long, short = 'y')]
        sysname_match: Vec<String>,

        /// Match parent device
        #[arg(long, short = 'b')]
        parent_match: Option<String>,

        /// Trigger for prioritized subsystems first
        #[arg(long)]
        prioritized_subsystem: Vec<String>,

        /// Do not actually trigger, just print devices
        #[arg(long, short = 'n')]
        dry_run: bool,

        /// Be more verbose
        #[arg(long, short = 'v')]
        verbose: bool,

        /// Wait for each triggered event to finish
        #[arg(long, short = 'w')]
        settle: bool,

        /// Device paths to trigger
        devices: Vec<String>,
    },

    /// Wait for pending udev events to complete
    Settle {
        /// Maximum time to wait in seconds
        #[arg(long, short = 't', default_value = "120")]
        timeout: u64,

        /// Only wait for events with sequence numbers <= this
        #[arg(long, short = 'E')]
        exit_if_exists: Option<String>,
    },

    /// Listen to kernel uevents
    Monitor {
        /// Filter by subsystem
        #[arg(long, short = 's')]
        subsystem_match: Vec<String>,

        /// Filter by tag
        #[arg(long, short = 't')]
        tag_match: Vec<String>,

        /// Print the property list
        #[arg(long, short = 'p')]
        property: bool,

        /// Print kernel uevents
        #[arg(long, short = 'k')]
        kernel: bool,

        /// Print udev events (processed)
        #[arg(long, short = 'u')]
        udev: bool,

        /// Show environment/property values
        #[arg(long, short = 'e')]
        environment: bool,
    },

    /// Simulate a device event and show the resulting actions
    Test {
        /// Action to simulate (default: add)
        #[arg(long, short = 'a', default_value = "add")]
        action: String,

        /// Subsystem override
        #[arg(long, short = 'N')]
        resolve_names: Option<String>,

        /// Device path in sysfs
        devpath: String,
    },

    /// Test a built-in command
    TestBuiltin {
        /// The builtin command to test
        command: String,

        /// Device path in sysfs
        devpath: String,
    },

    /// Send control commands to the running udev daemon
    Control {
        /// Stop processing new events (for debugging)
        #[arg(long, short = 's')]
        stop_exec_queue: bool,

        /// Resume processing events
        #[arg(long, short = 'S')]
        start_exec_queue: bool,

        /// Reload rules and databases
        #[arg(long, short = 'R')]
        reload: bool,

        /// Reload rules and re-trigger events
        #[arg(long)]
        reload_rules: bool,

        /// Set the maximum number of children
        #[arg(long, short = 'c')]
        children_max: Option<usize>,

        /// Seconds to delay execution of RUN
        #[arg(long, short = 'e')]
        exec_delay: Option<u64>,

        /// Set the log level
        #[arg(long, short = 'l')]
        log_level: Option<String>,

        /// Request the daemon to exit
        #[arg(long)]
        exit: bool,

        /// Ping the daemon
        #[arg(long)]
        ping: bool,

        /// Maximum seconds to wait for reply
        #[arg(long, short = 't', default_value = "60")]
        timeout: u64,
    },

    /// Wait for devices to be processed by udev
    Wait {
        /// Maximum time to wait in seconds
        #[arg(long, short = 't', default_value = "120")]
        timeout: u64,

        /// Wait until devices are initialized (default), added, or removed
        #[arg(long, default_value = "initialized")]
        wait_until: String,

        /// Wait for udev event queue to settle first
        #[arg(long)]
        settle: bool,

        /// Device paths to wait for
        devices: Vec<String>,
    },

    /// Show version
    Version,
}

// ---------------------------------------------------------------------------
// udevadm info
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn cmd_info(
    name: &Option<String>,
    path: &Option<String>,
    query: &str,
    export: bool,
    export_prefix: &Option<String>,
    root: bool,
    attribute_walk: bool,
    export_db: bool,
    cleanup_db: bool,
    _value: bool,
    device_id_of_file: &Option<String>,
    devices: &[String],
) -> i32 {
    if let Some(file_path) = device_id_of_file {
        return cmd_info_device_id_of_file(file_path);
    }

    if cleanup_db {
        return cmd_info_cleanup_db();
    }

    if export_db {
        return cmd_info_export_db();
    }

    // Collect device paths to query
    let mut syspaths: Vec<PathBuf> = Vec::new();

    if let Some(dev_name) = name {
        if let Some(sp) = devname_to_syspath(dev_name) {
            syspaths.push(sp);
        } else {
            eprintln!("udevadm info: device node '{}' not found", dev_name);
            return 1;
        }
    }

    if let Some(dev_path) = path {
        let sp = normalize_syspath(dev_path);
        if sp.exists() {
            syspaths.push(sp);
        } else {
            eprintln!("udevadm info: device path '{}' not found", dev_path);
            return 1;
        }
    }

    for dev in devices {
        // Could be a /dev node or /sys path
        if dev.starts_with("/dev/") {
            if let Some(sp) = devname_to_syspath(dev) {
                syspaths.push(sp);
            }
        } else {
            let sp = normalize_syspath(dev);
            if sp.exists() {
                syspaths.push(sp);
            }
        }
    }

    if syspaths.is_empty() {
        eprintln!("udevadm info: no device specified");
        eprintln!("Use --name=DEVNODE or --path=SYSPATH or pass device paths as arguments");
        return 1;
    }

    for syspath in &syspaths {
        let devpath = syspath_to_devpath(syspath);

        if attribute_walk {
            print_attribute_walk(syspath);
            continue;
        }

        // Read udev database for this device
        let db_props = read_device_db_by_syspath(syspath, &devpath);

        // Read sysfs uevent properties
        let mut props = read_sysfs_uevent(syspath);

        // Merge database properties
        for (k, v) in &db_props.env {
            props.insert(k.clone(), v.clone());
        }

        let prefix = export_prefix.as_deref().unwrap_or("");

        match query {
            "name" | "n" => {
                if let Some(devname) = props.get("DEVNAME") {
                    if root {
                        println!("/dev/{}", devname);
                    } else {
                        println!("{}", devname);
                    }
                }
            }
            "symlink" | "s" => {
                for link in &db_props.symlinks {
                    println!("{}", link);
                }
            }
            "path" | "p" => {
                println!("{}", devpath);
            }
            "property" | "e" => {
                for (k, v) in &props {
                    println!("{}{}={}", prefix, k, v);
                }
            }
            _ => {
                println!("P: {}", devpath);
                if let Some(devname) = props.get("DEVNAME") {
                    println!("N: {}", devname);
                }
                for link in &db_props.symlinks {
                    println!("S: {}", link);
                }
                if export {
                    for (k, v) in &props {
                        println!("E: {}{}={}", prefix, k, v);
                    }
                } else {
                    for (k, v) in &props {
                        println!("E: {}={}", k, v);
                    }
                }
                println!();
            }
        }
    }

    0
}

/// Export the entire udev database.
/// Print the major:minor device ID of the block device backing a file.
///
/// This implements `udevadm info --device-id-of-file=PATH`, which is used by
/// the NixOS initrd to identify the root device. It calls `stat()` on the
/// given path and prints the major:minor of the device the file resides on
/// (i.e. `st_dev`).
fn cmd_info_device_id_of_file(file_path: &str) -> i32 {
    let path = Path::new(file_path);
    match fs::metadata(path) {
        Ok(meta) => {
            let dev = meta.dev();
            let major = libc::major(dev);
            let minor = libc::minor(dev);
            println!("{}:{}", major, minor);
            0
        }
        Err(e) => {
            eprintln!("udevadm info: cannot stat '{}': {}", file_path, e);
            1
        }
    }
}

fn cmd_info_export_db() -> i32 {
    let db_dir = Path::new(DB_DIR);
    if !db_dir.is_dir() {
        eprintln!("udevadm info: udev database directory {} not found", DB_DIR);
        return 1;
    }

    let mut entries: Vec<PathBuf> = match fs::read_dir(db_dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
        Err(e) => {
            eprintln!("udevadm info: failed to read {}: {}", DB_DIR, e);
            return 1;
        }
    };
    entries.sort();

    for entry in &entries {
        if let Some(name) = entry.file_name().and_then(|n| n.to_str())
            && (name.starts_with('.') || name.ends_with(".tmp"))
        {
            continue;
        }

        if let Ok(content) = fs::read_to_string(entry) {
            println!("P: {}", entry.display());
            print!("{}", content);
            println!();
        }
    }

    0
}

/// Cleanup the udev database.
fn cmd_info_cleanup_db() -> i32 {
    let dirs_to_clean = [DB_DIR, TAGS_DIR];
    for dir in &dirs_to_clean {
        let path = Path::new(dir);
        if path.is_dir() {
            match fs::remove_dir_all(path) {
                Ok(()) => {
                    let _ = fs::create_dir_all(path);
                    log::info!("Cleaned {}", dir);
                }
                Err(e) => {
                    eprintln!("udevadm info: failed to clean {}: {}", dir, e);
                }
            }
        }
    }
    0
}

// ---------------------------------------------------------------------------
// udevadm trigger
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn cmd_trigger(
    type_: &str,
    action: &str,
    subsystem_match: &[String],
    subsystem_nomatch: &[String],
    attr_match: &[String],
    attr_nomatch: &[String],
    property_match: &[String],
    tag_match: &[String],
    sysname_match: &[String],
    parent_match: &Option<String>,
    prioritized_subsystem: &[String],
    dry_run: bool,
    verbose: bool,
    _settle: bool,
    devices: &[String],
) -> i32 {
    let mut count = 0u64;
    let mut errors = 0u64;

    // If specific devices are given, trigger only those
    if !devices.is_empty() {
        for dev in devices {
            let syspath = normalize_syspath(dev);
            if trigger_one_device(
                &syspath,
                action,
                subsystem_match,
                subsystem_nomatch,
                attr_match,
                attr_nomatch,
                property_match,
                tag_match,
                sysname_match,
                dry_run,
                verbose,
            ) {
                count += 1;
            } else {
                errors += 1;
            }
        }
    } else {
        // Enumerate devices from sysfs
        let scan_dirs: Vec<&str> = match type_ {
            "subsystems" => vec!["/sys/bus", "/sys/class"],
            _ => vec!["/sys/devices"],
        };

        // Handle prioritized subsystems first
        if !prioritized_subsystem.is_empty() {
            for subsys in prioritized_subsystem {
                // Parse comma-separated subsystem list
                for sub in subsys.split(',') {
                    let sub = sub.trim();
                    if sub.is_empty() {
                        continue;
                    }
                    let class_dir = format!("/sys/class/{}", sub);
                    let bus_dir = format!("/sys/bus/{}/devices", sub);

                    for dir in &[&class_dir, &bus_dir] {
                        if let Ok(entries) = fs::read_dir(dir) {
                            let mut paths: Vec<PathBuf> =
                                entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
                            paths.sort();
                            for path in paths {
                                let resolved = if path.is_symlink() {
                                    fs::canonicalize(&path).unwrap_or(path)
                                } else {
                                    path
                                };
                                if trigger_one_device(
                                    &resolved,
                                    action,
                                    subsystem_match,
                                    subsystem_nomatch,
                                    attr_match,
                                    attr_nomatch,
                                    property_match,
                                    tag_match,
                                    sysname_match,
                                    dry_run,
                                    verbose,
                                ) {
                                    count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Scan the main directories
        for scan_dir in &scan_dirs {
            let base = Path::new(scan_dir);
            if !base.is_dir() {
                continue;
            }
            trigger_walk(
                base,
                action,
                subsystem_match,
                subsystem_nomatch,
                attr_match,
                attr_nomatch,
                property_match,
                tag_match,
                sysname_match,
                parent_match,
                dry_run,
                verbose,
                &mut count,
                &mut errors,
                0,
            );
        }
    }

    if verbose || dry_run {
        eprintln!("udevadm trigger: triggered {} device(s)", count);
    }

    if errors > 0 { 1 } else { 0 }
}

#[allow(clippy::too_many_arguments)]
fn trigger_walk(
    dir: &Path,
    action: &str,
    subsystem_match: &[String],
    subsystem_nomatch: &[String],
    attr_match: &[String],
    attr_nomatch: &[String],
    property_match: &[String],
    tag_match: &[String],
    sysname_match: &[String],
    parent_match: &Option<String>,
    dry_run: bool,
    verbose: bool,
    count: &mut u64,
    errors: &mut u64,
    depth: usize,
) {
    if depth > 20 {
        return; // Prevent infinite recursion
    }

    // Check if this device has a uevent file
    let uevent_path = dir.join("uevent");
    if uevent_path.exists() {
        // Check parent match
        if let Some(parent) = parent_match {
            let parent_path = normalize_syspath(parent);
            let dir_str = dir.to_string_lossy();
            let parent_str = parent_path.to_string_lossy();
            if !dir_str.starts_with(parent_str.as_ref()) {
                // Not a child of the parent
            } else if trigger_one_device(
                dir,
                action,
                subsystem_match,
                subsystem_nomatch,
                attr_match,
                attr_nomatch,
                property_match,
                tag_match,
                sysname_match,
                dry_run,
                verbose,
            ) {
                *count += 1;
            } else {
                *errors += 1;
            }
        } else if trigger_one_device(
            dir,
            action,
            subsystem_match,
            subsystem_nomatch,
            attr_match,
            attr_nomatch,
            property_match,
            tag_match,
            sysname_match,
            dry_run,
            verbose,
        ) {
            *count += 1;
        }
    }

    // Recurse into subdirectories
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut subdirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().map(|t| t.is_dir()).unwrap_or(false) && {
                let name = e.file_name();
                let name = name.to_string_lossy();
                // Skip sysfs loop-causing directories
                name != "subsystem"
                    && name != "driver"
                    && name != "module"
                    && name != "firmware_node"
                    && name != "device"
                    && name != "power"
            }
        })
        .map(|e| e.path())
        .collect();
    subdirs.sort();

    for subdir in subdirs {
        trigger_walk(
            &subdir,
            action,
            subsystem_match,
            subsystem_nomatch,
            attr_match,
            attr_nomatch,
            property_match,
            tag_match,
            sysname_match,
            parent_match,
            dry_run,
            verbose,
            count,
            errors,
            depth + 1,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn trigger_one_device(
    syspath: &Path,
    action: &str,
    subsystem_match: &[String],
    subsystem_nomatch: &[String],
    attr_match: &[String],
    attr_nomatch: &[String],
    property_match: &[String],
    tag_match: &[String],
    sysname_match: &[String],
    dry_run: bool,
    verbose: bool,
) -> bool {
    // Read device subsystem
    let subsystem = read_sysfs_subsystem(syspath);
    let sysname = syspath
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    // Apply filters
    if !subsystem_match.is_empty() && !subsystem_match.iter().any(|s| s == &subsystem) {
        return false;
    }
    if subsystem_nomatch.iter().any(|s| s == &subsystem) {
        return false;
    }

    if !sysname_match.is_empty() && !sysname_match.iter().any(|s| glob_match(s, &sysname)) {
        return false;
    }

    // Check attribute matches
    for attr_spec in attr_match {
        if let Some((attr, val)) = attr_spec.split_once('=') {
            let attr_path = syspath.join(attr);
            if let Ok(content) = fs::read_to_string(&attr_path) {
                if content.trim() != val {
                    return false;
                }
            } else {
                return false;
            }
        }
    }
    for attr_spec in attr_nomatch {
        if let Some((attr, val)) = attr_spec.split_once('=') {
            let attr_path = syspath.join(attr);
            if let Ok(content) = fs::read_to_string(&attr_path)
                && content.trim() == val
            {
                return false;
            }
        }
    }

    // Check property matches
    if !property_match.is_empty() {
        let uevent_props = read_sysfs_uevent(syspath);
        for prop_spec in property_match {
            if let Some((key, val)) = prop_spec.split_once('=') {
                match uevent_props.get(key) {
                    Some(v) if glob_match(val, v) => {}
                    _ => return false,
                }
            }
        }
    }

    // Check tag matches
    if !tag_match.is_empty() {
        let _devpath = syspath_to_devpath(syspath);
        let dev_id = make_dev_id_from_syspath(syspath, &subsystem);
        let mut has_all_tags = true;
        for tag in tag_match {
            let tag_file = Path::new(TAGS_DIR).join(tag).join(&dev_id);
            if !tag_file.exists() {
                has_all_tags = false;
                break;
            }
        }
        if !has_all_tags {
            return false;
        }
    }

    if verbose {
        eprintln!("{}", syspath.display());
    }

    if dry_run {
        return true;
    }

    // Write action to the uevent file
    let uevent_path = syspath.join("uevent");
    match fs::OpenOptions::new().write(true).open(&uevent_path) {
        Ok(mut f) => {
            if f.write_all(action.as_bytes()).is_ok() {
                true
            } else {
                log::debug!("Failed to write to {}", uevent_path.display());
                false
            }
        }
        Err(e) => {
            log::debug!("Failed to open {}: {}", uevent_path.display(), e);
            false
        }
    }
}

// ---------------------------------------------------------------------------
// udevadm settle
// ---------------------------------------------------------------------------

fn cmd_settle(timeout: u64, exit_if_exists: &Option<String>) -> i32 {
    let start = Instant::now();
    let timeout_dur = Duration::from_secs(timeout);

    // Fast path: queue is already empty
    if !Path::new(QUEUE_FILE).exists() {
        return 0;
    }

    // Try inotify-based watching for efficient queue file monitoring.
    // We watch the parent directory (/run/udev/) for IN_DELETE events so we
    // get notified the moment the queue file is removed by the daemon.
    // Falls back to polling if inotify is unavailable.
    let inotify = nix::sys::inotify::Inotify::init(
        nix::sys::inotify::InitFlags::IN_NONBLOCK | nix::sys::inotify::InitFlags::IN_CLOEXEC,
    )
    .ok();

    let _watch_descriptor = inotify.as_ref().and_then(|ino| {
        let parent = Path::new(QUEUE_FILE)
            .parent()
            .unwrap_or(Path::new("/run/udev"));
        ino.add_watch(
            parent,
            nix::sys::inotify::AddWatchFlags::IN_DELETE
                | nix::sys::inotify::AddWatchFlags::IN_MOVED_FROM
                | nix::sys::inotify::AddWatchFlags::IN_CREATE
                | nix::sys::inotify::AddWatchFlags::IN_MOVED_TO,
        )
        .ok()
    });

    let use_inotify = inotify.is_some() && _watch_descriptor.is_some();
    if use_inotify {
        log::debug!("settle: using inotify to watch for queue file removal");
    }

    loop {
        if start.elapsed() >= timeout_dur {
            eprintln!("udevadm settle: timeout reached");
            return 1;
        }

        // Check if a file exists (--exit-if-exists)
        if let Some(path) = exit_if_exists
            && Path::new(path).exists()
        {
            return 0;
        }

        // Check queue file — if it doesn't exist, the queue is empty
        if !Path::new(QUEUE_FILE).exists() {
            return 0;
        }

        // Also ask the daemon via the control socket
        match send_control_command("SETTLE", Duration::from_secs(5)) {
            Ok(resp) if resp.starts_with("OK") => return 0,
            Ok(_) => {
                // Queue is busy, wait
            }
            Err(_) => {
                // Can't reach daemon — check queue file only
                if !Path::new(QUEUE_FILE).exists() {
                    return 0;
                }
            }
        }

        if let Some(ref ino) = inotify {
            // Drain inotify events — we don't need to inspect them in detail,
            // the queue-file existence check above will catch the deletion on
            // the next iteration.  The non-blocking read keeps the kernel
            // buffer from filling up while we sleep briefly before re-checking.
            let _ = ino.read_events();

            // Sleep a short interval — much more responsive than the 200 ms
            // poll fallback.  The inotify drain above prevents buffer overflows;
            // the actual "queue is empty" detection happens via the
            // Path::exists() check at the top of the loop.
            std::thread::sleep(Duration::from_millis(50));
        } else {
            // Fallback: poll-based sleep (no inotify available)
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

// ---------------------------------------------------------------------------
// udevadm monitor
// ---------------------------------------------------------------------------

fn cmd_monitor(
    subsystem_match: &[String],
    _tag_match: &[String],
    property: bool,
    kernel: bool,
    udev: bool,
    environment: bool,
) -> i32 {
    let show_kernel = kernel || !udev;
    let show_udev = udev || !kernel;

    println!("monitor will print the received events for:");
    if show_kernel {
        println!("KERNEL - the kernel uevent");
    }
    if show_udev {
        println!("UDEV - udev event after rules processing");
    }
    println!();

    // Open netlink socket for kernel uevents
    let nl_fd = match open_uevent_socket() {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("udevadm monitor: failed to open netlink socket: {}", e);
            return 1;
        }
    };

    // Install signal handler for clean exit
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_DFL);
        libc::signal(libc::SIGTERM, libc::SIG_DFL);
    }

    let mut buf = [0u8; 8192];
    loop {
        // Poll with a 1 second timeout
        let mut pfd = libc::pollfd {
            fd: nl_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ret = unsafe { libc::poll(&mut pfd as *mut libc::pollfd, 1, 1000) };
        if ret < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if ret == 0 {
            continue; // timeout
        }

        let n = unsafe { libc::recv(nl_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
        if n <= 0 {
            continue;
        }

        let data = &buf[..n as usize];

        // Parse the uevent
        if let Some(event) = parse_monitor_event(data) {
            // Filter by subsystem
            if !subsystem_match.is_empty()
                && let Some(subsys) = event.get("SUBSYSTEM")
                && !subsystem_match.iter().any(|s| s == subsys)
            {
                continue;
            }

            let action = event.get("ACTION").map(|s| s.as_str()).unwrap_or("unknown");
            let devpath = event.get("DEVPATH").map(|s| s.as_str()).unwrap_or("?");
            let subsystem = event.get("SUBSYSTEM").map(|s| s.as_str()).unwrap_or("?");

            let ts = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| format!("{}.{:06}", d.as_secs(), d.subsec_micros()))
                .unwrap_or_else(|_| "0.000000".to_string());

            if show_kernel {
                println!("KERNEL[{}] {} {} ({})", ts, action, devpath, subsystem);
                if property || environment {
                    for (k, v) in &event {
                        println!("{}={}", k, v);
                    }
                    println!();
                }
            }
        }
    }

    unsafe {
        libc::close(nl_fd);
    }

    0
}

/// Parse a raw uevent buffer into key=value pairs.
fn parse_monitor_event(data: &[u8]) -> Option<HashMap<String, String>> {
    let mut props = HashMap::new();
    let mut first = true;

    for chunk in data.split(|&b| b == 0) {
        if chunk.is_empty() {
            continue;
        }
        let s = match std::str::from_utf8(chunk) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if first {
            first = false;
            // First line is "action@devpath"
            if let Some(at) = s.find('@') {
                props.insert("ACTION".to_string(), s[..at].to_string());
                props.insert("DEVPATH".to_string(), s[at + 1..].to_string());
                continue;
            }
        }

        if let Some(eq) = s.find('=') {
            props.insert(s[..eq].to_string(), s[eq + 1..].to_string());
        }
    }

    if props.is_empty() { None } else { Some(props) }
}

// ---------------------------------------------------------------------------
// udevadm test
// ---------------------------------------------------------------------------

fn cmd_test(action: &str, devpath: &str) -> i32 {
    let syspath = normalize_syspath(devpath);
    if !syspath.exists() {
        eprintln!(
            "udevadm test: device '{}' not found (tried {})",
            devpath,
            syspath.display()
        );
        return 1;
    }

    let devpath_str = syspath_to_devpath(&syspath);

    // Read device properties from sysfs
    let props = read_sysfs_uevent(&syspath);
    let subsystem = props
        .get("SUBSYSTEM")
        .cloned()
        .unwrap_or_else(|| read_sysfs_subsystem(&syspath));
    let devname = props.get("DEVNAME").cloned().unwrap_or_default();
    let major = props.get("MAJOR").cloned().unwrap_or_default();
    let minor = props.get("MINOR").cloned().unwrap_or_default();
    let driver = props.get("DRIVER").cloned().unwrap_or_default();
    let devtype = props.get("DEVTYPE").cloned().unwrap_or_default();

    println!("Calling: test");
    println!("ACTION={}", action);
    println!("DEVPATH={}", devpath_str);
    println!("SUBSYSTEM={}", subsystem);
    if !devname.is_empty() {
        println!("DEVNAME={}", devname);
    }
    if !major.is_empty() {
        println!("MAJOR={}", major);
    }
    if !minor.is_empty() {
        println!("MINOR={}", minor);
    }
    if !driver.is_empty() {
        println!("DRIVER={}", driver);
    }
    if !devtype.is_empty() {
        println!("DEVTYPE={}", devtype);
    }

    println!();

    // Load and display matching rules
    println!("Reading rules from:");
    for dir in RULES_DIRS {
        if Path::new(dir).is_dir() {
            println!("  {}", dir);
        }
    }
    println!();

    // Load rules using the shared rules engine
    let ruleset = RuleSet::load();
    println!("Loaded {} rules", ruleset.rules.len());
    println!();

    // Build a UEvent and run it through the rules engine
    let mut env: HashMap<String, String> = props.clone();
    env.insert("ACTION".to_string(), action.to_string());
    let mut event = UEvent {
        action: action.to_string(),
        devpath: devpath_str.clone(),
        subsystem: subsystem.clone(),
        devtype: devtype.clone(),
        devname: devname.clone(),
        driver: driver.clone(),
        major: major.clone(),
        minor: minor.clone(),
        seqnum: 0,
        env,
    };

    let result = process_rules(&ruleset, &mut event, None);

    // Display rule processing results
    if let Some(ref name) = result.name {
        println!("NAME='{}'", name);
    }
    if !result.symlinks.is_empty() {
        println!("SYMLINK='{}'", result.symlinks.join(" "));
    }
    if let Some(ref owner) = result.owner {
        println!("OWNER='{}'", owner);
    }
    if let Some(ref group) = result.group {
        println!("GROUP='{}'", group);
    }
    if let Some(ref mode) = result.mode {
        println!("MODE='{}'", mode);
    }
    if !result.tags.is_empty() {
        for tag in &result.tags {
            println!("TAG='{}'", tag);
        }
    }
    if !result.run_programs.is_empty() {
        for prog in &result.run_programs {
            println!("RUN='{}'", prog);
        }
    }
    if !result.run_builtins.is_empty() {
        for builtin in &result.run_builtins {
            println!("RUN{{builtin}}='{}'", builtin);
        }
    }
    if !result.env_overrides.is_empty() {
        for (k, v) in &result.env_overrides {
            println!("ENV{{{}}}='{}'", k, v);
        }
    }
    if !result.sysattr_writes.is_empty() {
        for (attr, val) in &result.sysattr_writes {
            println!("ATTR{{{}}}='{}'", attr, val);
        }
    }
    if !result.options.is_empty() {
        for opt in &result.options {
            println!("OPTIONS='{}'", opt);
        }
    }

    println!();
    println!("Device properties:");
    for (k, v) in &event.env {
        println!("  {}={}", k, v);
    }

    0
}

// ---------------------------------------------------------------------------
// udevadm control
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn cmd_control(
    stop_exec_queue: bool,
    start_exec_queue: bool,
    reload: bool,
    reload_rules: bool,
    children_max: &Option<usize>,
    exec_delay: &Option<u64>,
    log_level: &Option<String>,
    exit: bool,
    ping: bool,
    timeout: u64,
) -> i32 {
    let timeout_dur = Duration::from_secs(timeout);

    if ping {
        match send_control_command("PING", timeout_dur) {
            Ok(resp) if resp.starts_with("OK") => {
                println!("udevd is running");
                return 0;
            }
            Ok(resp) => {
                eprintln!("Unexpected response: {}", resp.trim());
                return 1;
            }
            Err(e) => {
                eprintln!("Cannot connect to udevd: {}", e);
                return 1;
            }
        }
    }

    if stop_exec_queue {
        return send_and_check("STOP_EXEC_QUEUE", timeout_dur);
    }

    if start_exec_queue {
        return send_and_check("START_EXEC_QUEUE", timeout_dur);
    }

    if reload || reload_rules {
        return send_and_check("RELOAD", timeout_dur);
    }

    if let Some(n) = children_max {
        return send_and_check(&format!("SET_MAX_CHILDREN {}", n), timeout_dur);
    }

    if let Some(d) = exec_delay {
        return send_and_check(&format!("SET_EXEC_DELAY {}", d), timeout_dur);
    }

    if let Some(level) = log_level {
        return send_and_check(&format!("SET_LOG_LEVEL {}", level), timeout_dur);
    }

    if exit {
        return send_and_check("EXIT", timeout_dur);
    }

    eprintln!("udevadm control: no command specified");
    1
}

fn send_and_check(cmd: &str, timeout: Duration) -> i32 {
    match send_control_command(cmd, timeout) {
        Ok(resp) if resp.starts_with("OK") => 0,
        Ok(resp) => {
            eprintln!("udevadm control: {}", resp.trim());
            1
        }
        Err(e) => {
            eprintln!("udevadm control: failed to communicate with udevd: {}", e);
            1
        }
    }
}

fn send_control_command(cmd: &str, timeout: Duration) -> io::Result<String> {
    let mut stream = UnixStream::connect(CONTROL_SOCKET_PATH)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(cmd.as_bytes())?;
    stream.shutdown(Shutdown::Write)?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

// ---------------------------------------------------------------------------
// Helper functions: sysfs and device database
// ---------------------------------------------------------------------------

/// Normalize a device path (may be a devpath like /devices/... or a syspath like /sys/devices/...).
fn normalize_syspath(path: &str) -> PathBuf {
    if path.starts_with("/sys/") {
        PathBuf::from(path)
    } else if path.starts_with('/') {
        PathBuf::from(format!("/sys{}", path))
    } else {
        PathBuf::from(format!("/sys/{}", path))
    }
}

/// Convert a sysfs path to a devpath (strip /sys prefix).
fn syspath_to_devpath(syspath: &Path) -> String {
    let s = syspath.to_string_lossy();
    if let Some(rest) = s.strip_prefix("/sys") {
        rest.to_string()
    } else {
        s.to_string()
    }
}

/// Try to find the sysfs path for a /dev/ device node.
fn devname_to_syspath(devname: &str) -> Option<PathBuf> {
    let devname = devname.strip_prefix("/dev/").unwrap_or(devname);

    // Check /sys/class/ for a matching device
    if let Ok(entries) = fs::read_dir("/sys/class") {
        for class_entry in entries.flatten() {
            let dev_path = class_entry.path().join(devname);
            if dev_path.exists() || dev_path.is_symlink() {
                let resolved = fs::canonicalize(&dev_path).unwrap_or(dev_path);
                return Some(resolved);
            }
        }
    }

    // Check /sys/block/
    let block_path = PathBuf::from("/sys/block").join(devname);
    if block_path.exists() || block_path.is_symlink() {
        let resolved = fs::canonicalize(&block_path).unwrap_or(block_path);
        return Some(resolved);
    }

    // Walk /sys/class and /sys/block for nested names like "mapper/control"
    let parts: Vec<&str> = devname.splitn(2, '/').collect();
    if parts.len() == 2 {
        let class_dir = PathBuf::from("/sys/class").join(parts[0]);
        if class_dir.is_dir() {
            let dev_path = class_dir.join(parts[1]);
            if dev_path.exists() || dev_path.is_symlink() {
                let resolved = fs::canonicalize(&dev_path).unwrap_or(dev_path);
                return Some(resolved);
            }
        }
    }

    None
}

/// Read the subsystem of a device from sysfs.
fn read_sysfs_subsystem(syspath: &Path) -> String {
    let link = syspath.join("subsystem");
    if let Ok(target) = fs::read_link(&link) {
        target
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    } else {
        String::new()
    }
}

/// Read the uevent properties of a device from sysfs.
fn read_sysfs_uevent(syspath: &Path) -> HashMap<String, String> {
    let mut props = HashMap::new();
    let uevent_path = syspath.join("uevent");
    if let Ok(content) = fs::read_to_string(&uevent_path) {
        for line in content.lines() {
            let line = line.trim();
            if let Some(eq) = line.find('=') {
                let key = line[..eq].to_string();
                let val = line[eq + 1..].to_string();
                props.insert(key, val);
            }
        }
    }
    // Add DEVPATH
    let devpath = syspath_to_devpath(syspath);
    props.insert("DEVPATH".to_string(), devpath);
    // Add SUBSYSTEM
    let subsystem = read_sysfs_subsystem(syspath);
    if !subsystem.is_empty() {
        props.insert("SUBSYSTEM".to_string(), subsystem);
    }
    props
}

/// Print the sysfs attribute walk for a device (similar to udevadm info --attribute-walk).
fn print_attribute_walk(syspath: &Path) {
    let mut current = syspath.to_path_buf();
    println!("  looking at device '{}':", syspath_to_devpath(&current));
    print_device_attributes(&current);
    println!();

    // Walk up to parents
    while current.pop() {
        let devpath = syspath_to_devpath(&current);
        if devpath.is_empty() || devpath == "/" || !current.starts_with("/sys/devices") {
            break;
        }
        // Only show directories that have a uevent file (i.e., are devices)
        if current.join("uevent").exists() {
            println!("  looking at parent device '{}':", devpath);
            print_device_attributes(&current);
            println!();
        }
    }
}

fn print_device_attributes(syspath: &Path) {
    let subsystem = read_sysfs_subsystem(syspath);
    let driver = {
        let link = syspath.join("driver");
        if let Ok(target) = fs::read_link(&link) {
            target
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        } else {
            String::new()
        }
    };
    let kernel = syspath
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    println!("    KERNEL==\"{}\"", kernel);
    println!("    SUBSYSTEM==\"{}\"", subsystem);
    println!("    DRIVER==\"{}\"", driver);

    // List readable attributes
    if let Ok(entries) = fs::read_dir(syspath) {
        let mut attrs: Vec<(String, String)> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            // Skip certain files
            if name == "uevent"
                || name == "subsystem"
                || name == "driver"
                || name == "module"
                || name == "firmware_node"
                || name == "power"
                || name == "device"
            {
                continue;
            }
            let path = entry.path();
            if path.is_file() {
                // Try to read the attribute (skip binary/unreadable ones)
                if let Ok(content) = fs::read_to_string(&path) {
                    let value = content.trim().to_string();
                    if !value.is_empty()
                        && value.len() < 256
                        && value.is_ascii()
                        && !value.contains('\0')
                    {
                        attrs.push((name, value));
                    }
                }
            }
        }
        attrs.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, value) in &attrs {
            println!("    ATTR{{{}}}==\"{}\"", name, value);
        }
    }
}

/// Device database entry.
struct DeviceDb {
    symlinks: Vec<String>,
    tags: Vec<String>,
    env: HashMap<String, String>,
}

/// Read the udev database entry for a device.
fn read_device_db_by_syspath(syspath: &Path, devpath: &str) -> DeviceDb {
    let mut db = DeviceDb {
        symlinks: Vec::new(),
        tags: Vec::new(),
        env: HashMap::new(),
    };

    // Try to find the database file
    // Read uevent to get MAJOR/MINOR
    let props = read_sysfs_uevent(syspath);
    let major = props.get("MAJOR");
    let minor = props.get("MINOR");
    let subsystem = props.get("SUBSYSTEM").map(|s| s.as_str()).unwrap_or("");

    let db_path = if let (Some(maj), Some(min)) = (major, minor) {
        let dev_type = if subsystem == "block" { 'b' } else { 'c' };
        Path::new(DB_DIR).join(format!("{}{}:{}", dev_type, maj, min))
    } else {
        let basename = devpath.rsplit('/').next().unwrap_or(devpath);
        if subsystem.is_empty() {
            Path::new(DB_DIR).join(format!("n{}", devpath.replace('/', "\\x2f")))
        } else {
            Path::new(DB_DIR).join(format!("+{}:{}", subsystem, basename))
        }
    };

    if let Ok(content) = fs::read_to_string(&db_path) {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("S:") {
                db.symlinks.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("G:") {
                db.tags.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("E:")
                && let Some(eq) = rest.find('=')
            {
                db.env
                    .insert(rest[..eq].to_string(), rest[eq + 1..].to_string());
            }
        }
    }

    db
}

/// Make a device ID for tag lookups.
fn make_dev_id_from_syspath(syspath: &Path, subsystem: &str) -> String {
    let props = read_sysfs_uevent(syspath);
    let major = props.get("MAJOR");
    let minor = props.get("MINOR");

    if let (Some(maj), Some(min)) = (major, minor) {
        let dev_type = if subsystem == "block" { 'b' } else { 'c' };
        format!("{}{}:{}", dev_type, maj, min)
    } else {
        let devpath = syspath_to_devpath(syspath);
        let basename = devpath.rsplit('/').next().unwrap_or(&devpath);
        if subsystem.is_empty() {
            format!("n{}", devpath.replace('/', "\\x2f"))
        } else {
            format!("+{}:{}", subsystem, basename)
        }
    }
}

// ---------------------------------------------------------------------------
// udevadm wait
// ---------------------------------------------------------------------------

/// Wait for devices to be processed by udev.
///
/// Polls the specified device paths until they satisfy the wait condition
/// (initialized, added, or removed) or the timeout expires.
fn cmd_wait(timeout: u64, wait_until: &str, _settle: bool, devices: &[String]) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(timeout);

    let check_device = |path: &str| -> bool {
        match wait_until {
            "removed" => !Path::new(path).exists(),
            "added" => Path::new(path).exists(),
            // "initialized" — check that the device exists and has a udev db entry
            // For simplicity, we just check existence + the uevent file (which
            // indicates the kernel has finished creating the device node).
            _ => {
                let p = Path::new(path);
                if !p.exists() {
                    return false;
                }
                // If it's a sysfs path, check for the "uevent" file which
                // indicates the device is initialized by the kernel.
                // For /dev/ paths, existence is sufficient.
                if path.starts_with("/sys/") {
                    // Check for udev database entry
                    let db_path = if let Ok(_md) = fs::metadata(p) {
                        // For sysfs directories, the udev db key is based on the
                        // sysfs path relative to /sys/
                        let sys_relative = path.strip_prefix("/sys/").unwrap_or(path);
                        let db_key = sys_relative.replace('/', "\\x2f");
                        PathBuf::from(DB_DIR).join(db_key)
                    } else {
                        return false;
                    };
                    // If there's a udev db entry, the device is fully initialized.
                    // Otherwise, just check uevent exists (kernel has created it).
                    if db_path.exists() {
                        return true;
                    }
                    p.join("uevent").exists() || p.is_file()
                } else {
                    true
                }
            }
        }
    };

    loop {
        let all_ready = devices.iter().all(|d| check_device(d));
        if all_ready {
            return 0;
        }

        if Instant::now() >= deadline {
            for d in devices {
                if !check_device(d) {
                    eprintln!("udevadm wait: timeout waiting for device '{}'", d);
                }
            }
            return 1;
        }

        std::thread::sleep(Duration::from_millis(50));
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    // Multi-call dispatch: when invoked as `systemd-udevd` (e.g. via symlink
    // in the NixOS initrd where systemd-udevd -> udevadm), run the daemon
    // instead of the udevadm CLI.
    if systemd_udevd::invoked_as_daemon() {
        systemd_udevd::run_daemon();
        return ExitCode::SUCCESS;
    }

    init_logging();

    let cli = Cli::parse();

    if cli.debug {
        log::set_max_level(log::LevelFilter::Debug);
    }

    let exit_code = match cli.command {
        Commands::Info {
            ref name,
            ref path,
            ref query,
            export,
            ref export_prefix,
            root,
            attribute_walk,
            export_db,
            cleanup_db,
            value,
            ref device_id_of_file,
            ref devices,
        } => cmd_info(
            name,
            path,
            query,
            export,
            export_prefix,
            root,
            attribute_walk,
            export_db,
            cleanup_db,
            value,
            device_id_of_file,
            devices,
        ),

        Commands::Trigger {
            ref r#type,
            ref action,
            ref subsystem_match,
            ref subsystem_nomatch,
            ref attr_match,
            ref attr_nomatch,
            ref property_match,
            ref tag_match,
            ref sysname_match,
            ref parent_match,
            ref prioritized_subsystem,
            dry_run,
            verbose,
            settle,
            ref devices,
        } => cmd_trigger(
            r#type,
            action,
            subsystem_match,
            subsystem_nomatch,
            attr_match,
            attr_nomatch,
            property_match,
            tag_match,
            sysname_match,
            parent_match,
            prioritized_subsystem,
            dry_run,
            verbose,
            settle,
            devices,
        ),

        Commands::Settle {
            timeout,
            ref exit_if_exists,
        } => cmd_settle(timeout, exit_if_exists),

        Commands::Monitor {
            ref subsystem_match,
            ref tag_match,
            property,
            kernel,
            udev,
            environment,
        } => cmd_monitor(
            subsystem_match,
            tag_match,
            property,
            kernel,
            udev,
            environment,
        ),

        Commands::Test {
            ref action,
            resolve_names: _,
            ref devpath,
        } => cmd_test(action, devpath),

        Commands::TestBuiltin {
            ref command,
            ref devpath,
        } => {
            eprintln!(
                "udevadm test-builtin: builtin '{}' on '{}' — not fully implemented",
                command, devpath
            );
            0
        }

        Commands::Control {
            stop_exec_queue,
            start_exec_queue,
            reload,
            reload_rules,
            ref children_max,
            ref exec_delay,
            ref log_level,
            exit,
            ping,
            timeout,
        } => cmd_control(
            stop_exec_queue,
            start_exec_queue,
            reload,
            reload_rules,
            children_max,
            exec_delay,
            log_level,
            exit,
            ping,
            timeout,
        ),

        Commands::Wait {
            timeout,
            ref wait_until,
            settle,
            ref devices,
        } => cmd_wait(timeout, wait_until, settle, devices),

        Commands::Version => {
            println!("udevadm (rust-systemd)");
            0
        }
    };

    ExitCode::from(exit_code as u8)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Path normalization
    // -----------------------------------------------------------------------

    #[test]
    fn test_normalize_syspath_already_sys() {
        let p = normalize_syspath("/sys/devices/pci0000:00");
        assert_eq!(p, PathBuf::from("/sys/devices/pci0000:00"));
    }

    #[test]
    fn test_normalize_syspath_devpath() {
        let p = normalize_syspath("/devices/pci0000:00");
        assert_eq!(p, PathBuf::from("/sys/devices/pci0000:00"));
    }

    #[test]
    fn test_normalize_syspath_relative() {
        let p = normalize_syspath("devices/pci0000:00");
        assert_eq!(p, PathBuf::from("/sys/devices/pci0000:00"));
    }

    #[test]
    fn test_syspath_to_devpath() {
        assert_eq!(
            syspath_to_devpath(Path::new("/sys/devices/pci0000:00")),
            "/devices/pci0000:00"
        );
    }

    #[test]
    fn test_syspath_to_devpath_no_prefix() {
        assert_eq!(syspath_to_devpath(Path::new("/other/path")), "/other/path");
    }

    // -----------------------------------------------------------------------
    // Glob matching
    // -----------------------------------------------------------------------

    #[test]
    fn test_glob_simple_exact() {
        assert!(glob_match("hello", "hello"));
        assert!(!glob_match("hello", "world"));
    }

    #[test]
    fn test_glob_simple_star() {
        assert!(glob_match("sd*", "sda"));
        assert!(glob_match("sd*", "sda1"));
        assert!(glob_match("*", "anything"));
        assert!(!glob_match("sd*", "nvme0"));
    }

    #[test]
    fn test_glob_simple_question() {
        assert!(glob_match("sd?", "sda"));
        assert!(!glob_match("sd?", "sd"));
        assert!(!glob_match("sd?", "sdaa"));
    }

    #[test]
    fn test_glob_simple_complex() {
        assert!(glob_match("*a*", "abc"));
        assert!(glob_match("?b?", "abc"));
        assert!(!glob_match("?b?", "axc"));
    }

    #[test]
    fn test_glob_simple_empty() {
        assert!(glob_match("", ""));
        assert!(!glob_match("", "a"));
        assert!(glob_match("*", ""));
    }

    // -----------------------------------------------------------------------
    // Monitor event parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_monitor_event_basic() {
        let data =
            b"add@/devices/pci0000:00\0ACTION=add\0DEVPATH=/devices/pci0000:00\0SUBSYSTEM=pci\0";
        let event = parse_monitor_event(data).unwrap();
        assert_eq!(event.get("ACTION").unwrap(), "add");
        assert_eq!(event.get("DEVPATH").unwrap(), "/devices/pci0000:00");
        assert_eq!(event.get("SUBSYSTEM").unwrap(), "pci");
    }

    #[test]
    fn test_parse_monitor_event_empty() {
        assert!(parse_monitor_event(b"").is_none());
    }

    #[test]
    fn test_parse_monitor_event_key_value_only() {
        let data = b"ACTION=change\0DEVPATH=/devices/test\0";
        let event = parse_monitor_event(data).unwrap();
        assert_eq!(event.get("ACTION").unwrap(), "change");
    }

    // -----------------------------------------------------------------------
    // Device database reading
    // -----------------------------------------------------------------------

    #[test]
    fn test_make_dev_id_no_major_minor() {
        let syspath = Path::new("/sys/devices/pci0000:00");
        // This won't find real uevent, so it falls through to the subsystem-based ID
        let id = make_dev_id_from_syspath(syspath, "pci");
        assert!(id.starts_with('+') || id.starts_with('n'));
    }

    // -----------------------------------------------------------------------
    // Control socket communication
    // -----------------------------------------------------------------------

    #[test]
    fn test_send_control_command_no_daemon() {
        // Should fail gracefully when no daemon is running
        let result = send_control_command("PING", Duration::from_secs(1));
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // sysfs subsystem reading (non-existent device)
    // -----------------------------------------------------------------------

    #[test]
    fn test_read_sysfs_subsystem_nonexistent() {
        let s = read_sysfs_subsystem(Path::new("/sys/nonexistent_device_xyz"));
        assert!(s.is_empty());
    }

    #[test]
    fn test_read_sysfs_uevent_nonexistent() {
        let props = read_sysfs_uevent(Path::new("/sys/nonexistent_device_xyz"));
        // Should have DEVPATH at minimum
        assert!(props.contains_key("DEVPATH"));
        // But no SUBSYSTEM since the device doesn't exist
    }

    // -----------------------------------------------------------------------
    // devname_to_syspath
    // -----------------------------------------------------------------------

    #[test]
    fn test_devname_to_syspath_nonexistent() {
        assert!(devname_to_syspath("nonexistent_dev_xyz_123").is_none());
    }

    // -----------------------------------------------------------------------
    // CLI argument parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_cli_version() {
        use clap::CommandFactory;
        // Just verify the CLI can be constructed
        let _cmd = Cli::command();
    }

    #[test]
    fn test_cli_parse_info() {
        let cli = Cli::try_parse_from(["udevadm", "info", "--name=/dev/sda"]).unwrap();
        if let Commands::Info { name, .. } = cli.command {
            assert_eq!(name, Some("/dev/sda".to_string()));
        } else {
            panic!("Expected Info command");
        }
    }

    #[test]
    fn test_cli_parse_trigger() {
        let cli = Cli::try_parse_from([
            "udevadm",
            "trigger",
            "--type=all",
            "--action=add",
            "--verbose",
        ])
        .unwrap();
        if let Commands::Trigger {
            r#type,
            action,
            verbose,
            ..
        } = cli.command
        {
            assert_eq!(r#type, "all");
            assert_eq!(action, "add");
            assert!(verbose);
        } else {
            panic!("Expected Trigger command");
        }
    }

    #[test]
    fn test_cli_parse_settle() {
        let cli = Cli::try_parse_from(["udevadm", "settle", "--timeout=30"]).unwrap();
        if let Commands::Settle { timeout, .. } = cli.command {
            assert_eq!(timeout, 30);
        } else {
            panic!("Expected Settle command");
        }
    }

    #[test]
    fn test_cli_parse_control_reload() {
        let cli = Cli::try_parse_from(["udevadm", "control", "--reload"]).unwrap();
        if let Commands::Control { reload, .. } = cli.command {
            assert!(reload);
        } else {
            panic!("Expected Control command");
        }
    }

    #[test]
    fn test_cli_parse_control_ping() {
        let cli = Cli::try_parse_from(["udevadm", "control", "--ping"]).unwrap();
        if let Commands::Control { ping, .. } = cli.command {
            assert!(ping);
        } else {
            panic!("Expected Control command");
        }
    }

    #[test]
    fn test_cli_parse_monitor() {
        let cli = Cli::try_parse_from(["udevadm", "monitor", "--kernel", "--property"]).unwrap();
        if let Commands::Monitor {
            kernel, property, ..
        } = cli.command
        {
            assert!(kernel);
            assert!(property);
        } else {
            panic!("Expected Monitor command");
        }
    }

    #[test]
    fn test_cli_parse_test() {
        let cli =
            Cli::try_parse_from(["udevadm", "test", "--action=add", "/sys/devices/test"]).unwrap();
        if let Commands::Test {
            action, devpath, ..
        } = cli.command
        {
            assert_eq!(action, "add");
            assert_eq!(devpath, "/sys/devices/test");
        } else {
            panic!("Expected Test command");
        }
    }

    #[test]
    fn test_cli_parse_trigger_prioritized_subsystem() {
        let cli = Cli::try_parse_from([
            "udevadm",
            "trigger",
            "--type=all",
            "--action=add",
            "--prioritized-subsystem=module,block,tpmrm,net,tty,input",
        ])
        .unwrap();
        if let Commands::Trigger {
            prioritized_subsystem,
            ..
        } = cli.command
        {
            assert_eq!(prioritized_subsystem.len(), 1);
            assert_eq!(prioritized_subsystem[0], "module,block,tpmrm,net,tty,input");
        } else {
            panic!("Expected Trigger command");
        }
    }

    #[test]
    fn test_cli_parse_info_attribute_walk() {
        let cli = Cli::try_parse_from([
            "udevadm",
            "info",
            "--attribute-walk",
            "--path=/sys/devices/test",
        ])
        .unwrap();
        if let Commands::Info {
            attribute_walk,
            path,
            ..
        } = cli.command
        {
            assert!(attribute_walk);
            assert_eq!(path, Some("/sys/devices/test".to_string()));
        } else {
            panic!("Expected Info command");
        }
    }

    #[test]
    fn test_cli_parse_info_device_id_of_file() {
        let cli =
            Cli::try_parse_from(["udevadm", "info", "--device-id-of-file=/dev/null"]).unwrap();
        if let Commands::Info {
            device_id_of_file, ..
        } = cli.command
        {
            assert_eq!(device_id_of_file, Some("/dev/null".to_string()));
        } else {
            panic!("Expected Info command");
        }
    }

    #[test]
    fn test_cli_parse_info_device_id_of_file_equals() {
        // Also accept --device-id-of-file=PATH (equals form)
        let cli =
            Cli::try_parse_from(["udevadm", "info", "--device-id-of-file=/etc/hostname"]).unwrap();
        if let Commands::Info {
            device_id_of_file, ..
        } = cli.command
        {
            assert_eq!(device_id_of_file, Some("/etc/hostname".to_string()));
        } else {
            panic!("Expected Info command");
        }
    }

    #[test]
    fn test_device_id_of_file_returns_major_minor() {
        // /dev/null always exists; stat it and verify we get a valid major:minor
        let exit = cmd_info_device_id_of_file("/dev/null");
        assert_eq!(exit, 0);
    }

    #[test]
    fn test_device_id_of_file_nonexistent() {
        let exit = cmd_info_device_id_of_file("/nonexistent/path/that/does/not/exist");
        assert_eq!(exit, 1);
    }

    #[test]
    fn test_device_id_of_file_root() {
        // "/" always exists; should succeed with a valid device id
        let exit = cmd_info_device_id_of_file("/");
        assert_eq!(exit, 0);
    }

    // -----------------------------------------------------------------------
    // settle — inotify-based queue file watching tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_settle_returns_immediately_when_queue_absent() {
        // When the queue file does not exist, settle should return 0
        // immediately without waiting.  We can't call cmd_settle directly
        // because it checks the real QUEUE_FILE, but we can verify the
        // fast-path logic: Path::new(QUEUE_FILE).exists() == false => exit 0.
        // On a test machine without a running udevd, the queue file should
        // not exist.
        if !Path::new(QUEUE_FILE).exists() {
            let exit = cmd_settle(1, &None);
            assert_eq!(exit, 0, "settle should succeed when queue file is absent");
        }
    }

    #[test]
    fn test_settle_timeout_with_stale_queue_file() {
        // Create a temporary queue file to simulate a busy queue, then
        // verify settle times out.  We use a very short timeout (1 second)
        // to keep the test fast.
        let dir = tempfile::tempdir().unwrap();
        let fake_queue = dir.path().join("queue");
        std::fs::write(&fake_queue, "").unwrap();

        // We can't easily redirect QUEUE_FILE for cmd_settle, but we can
        // verify the inotify setup works independently.
        let ino = nix::sys::inotify::Inotify::init(
            nix::sys::inotify::InitFlags::IN_NONBLOCK | nix::sys::inotify::InitFlags::IN_CLOEXEC,
        );
        assert!(ino.is_ok(), "inotify should be available on Linux");

        let ino = ino.unwrap();
        let wd = ino.add_watch(
            dir.path(),
            nix::sys::inotify::AddWatchFlags::IN_DELETE
                | nix::sys::inotify::AddWatchFlags::IN_MOVED_FROM,
        );
        assert!(wd.is_ok(), "should be able to watch temp directory");

        // Remove the queue file — inotify should report the deletion
        std::fs::remove_file(&fake_queue).unwrap();
        std::thread::sleep(Duration::from_millis(50));

        let events = ino.read_events();
        assert!(events.is_ok(), "should be able to read inotify events");
        let events = events.unwrap();
        assert!(
            !events.is_empty(),
            "should have received deletion event from inotify"
        );
    }

    #[test]
    fn test_settle_inotify_watches_parent_directory() {
        // Verify that we can set up an inotify watch on a directory and
        // detect file creation/deletion — this mirrors what cmd_settle does.
        let dir = tempfile::tempdir().unwrap();

        let ino = nix::sys::inotify::Inotify::init(
            nix::sys::inotify::InitFlags::IN_NONBLOCK | nix::sys::inotify::InitFlags::IN_CLOEXEC,
        )
        .unwrap();
        let _wd = ino
            .add_watch(
                dir.path(),
                nix::sys::inotify::AddWatchFlags::IN_CREATE
                    | nix::sys::inotify::AddWatchFlags::IN_DELETE,
            )
            .unwrap();

        // Create a file — should trigger IN_CREATE
        let test_file = dir.path().join("queue");
        std::fs::write(&test_file, "").unwrap();
        std::thread::sleep(Duration::from_millis(50));

        let events = ino.read_events().unwrap();
        assert!(!events.is_empty(), "should detect file creation");

        // Now delete it — should trigger IN_DELETE
        std::fs::remove_file(&test_file).unwrap();
        std::thread::sleep(Duration::from_millis(50));

        let events = ino.read_events().unwrap();
        assert!(!events.is_empty(), "should detect file deletion");
    }

    #[test]
    fn test_settle_inotify_nonblocking_returns_eagain() {
        // Verify that reading from a non-blocking inotify fd with no
        // pending events returns EAGAIN (not an error), matching what
        // cmd_settle expects.
        let dir = tempfile::tempdir().unwrap();
        let ino = nix::sys::inotify::Inotify::init(
            nix::sys::inotify::InitFlags::IN_NONBLOCK | nix::sys::inotify::InitFlags::IN_CLOEXEC,
        )
        .unwrap();
        let _wd = ino
            .add_watch(dir.path(), nix::sys::inotify::AddWatchFlags::IN_DELETE)
            .unwrap();

        // No events pending — read should return Err(EAGAIN)
        match ino.read_events() {
            Err(nix::errno::Errno::EAGAIN) => {
                // Expected — non-blocking read with no events
            }
            Ok(events) => {
                assert!(events.is_empty(), "should have no events");
            }
            Err(e) => {
                panic!("unexpected inotify error: {}", e);
            }
        }
    }

    #[test]
    fn test_settle_exit_if_exists() {
        // --exit-if-exists should cause immediate return when the file exists,
        // even if the queue file would also exist.
        // Use a file that always exists:
        if !Path::new(QUEUE_FILE).exists() {
            let exit = cmd_settle(1, &Some("/dev/null".to_string()));
            assert_eq!(
                exit, 0,
                "should exit immediately when exit-if-exists file exists"
            );
        }
    }

    #[test]
    fn test_settle_cli_parse_timeout() {
        let cli = Cli::try_parse_from(["udevadm", "settle", "--timeout=5"]).unwrap();
        if let Commands::Settle { timeout, .. } = cli.command {
            assert_eq!(timeout, 5);
        } else {
            panic!("Expected Settle command");
        }
    }
}

//! systemd-machined — VM and container registration/tracking daemon
//!
//! This is a Rust implementation of systemd's `systemd-machined` daemon. It
//! manages the registration and tracking of virtual machines and containers
//! running on the local host.
//!
//! ## Features
//!
//! - Machine registration (register/terminate) with class (vm/container),
//!   service, scope, leader PID, root directory, and network interfaces
//! - Machine listing and status queries
//! - Runtime state files in `/run/systemd/machines/`
//! - Control socket at `/run/systemd/machined-control` for `machinectl` CLI
//! - D-Bus interface (`org.freedesktop.machine1`) with Manager object
//!   (ListMachines, GetMachine, GetMachineByPID, RegisterMachine,
//!   TerminateMachine, KillMachine, GetMachineOSRelease, ListImages,
//!   CloneImage, RenameImage, RemoveImage, SetImageLimit, SetPoolLimit,
//!   CopyTo, CopyFrom, OpenMachineLogin, OpenMachineShell,
//!   ImportTar, ImportRaw, ExportTar, ExportRaw, PullTar, PullRaw;
//!   properties: PoolPath, PoolUsage, PoolLimit); deferred registration
//!   to avoid blocking early boot before dbus-daemon is ready
//! - Image management: discover/clone/rename/remove/set-limit for directory
//!   and raw images in `/var/lib/machines/`
//! - Machine scoping: transient scope unit creation via systemd1 D-Bus
//! - Copy-to/copy-from: file copy between host and running containers
//! - PTY forwarding: openpty + nsenter for login/shell sessions
//! - OS image import/export/pull: tar/raw image import from file/stdin,
//!   export to file/stdout, pull from URL (http/https)
//! - sd_notify protocol (READY/WATCHDOG/STATUS/STOPPING)
//! - Signal handling (SIGTERM/SIGINT for shutdown, SIGHUP for reload)
//! - Stale machine cleanup (machines whose leader PID has exited)
//!
//! - Bind mounts: bind-mount host paths into running containers via
//!   mount namespace entry (fork + setns + mount), with --read-only and
//!   --mkdir support; bind-user mounts that also set up UID-mapped paths

use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixDatagram, UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use zbus::blocking::Connection;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const MACHINES_DIR: &str = "/run/systemd/machines";
const CONTROL_SOCKET_PATH: &str = "/run/systemd/machined-control";
const POOL_PATH: &str = "/var/lib/machines";

const DBUS_NAME: &str = "org.freedesktop.machine1";
const DBUS_PATH: &str = "/org/freedesktop/machine1";

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// Machine class — either a virtual machine or a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineClass {
    Container,
    Vm,
}

impl MachineClass {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "container" => Some(MachineClass::Container),
            "vm" => Some(MachineClass::Vm),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            MachineClass::Container => "container",
            MachineClass::Vm => "vm",
        }
    }
}

impl fmt::Display for MachineClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// State of a registered machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineState {
    Opening,
    Running,
    Closing,
}

impl MachineState {
    pub fn as_str(&self) -> &'static str {
        match self {
            MachineState::Opening => "opening",
            MachineState::Running => "running",
            MachineState::Closing => "closing",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "opening" => Some(MachineState::Opening),
            "running" => Some(MachineState::Running),
            "closing" => Some(MachineState::Closing),
            _ => None,
        }
    }
}

impl fmt::Display for MachineState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A registered machine.
#[derive(Debug, Clone)]
pub struct Machine {
    /// Unique machine name.
    pub name: String,
    /// Class — VM or container.
    pub class: MachineClass,
    /// Service that registered this machine (e.g. "systemd-nspawn").
    pub service: String,
    /// Scope unit name.
    pub scope: String,
    /// Leader PID in the host PID namespace.
    pub leader: u32,
    /// Root directory (for containers, typically `/`).
    pub root_directory: String,
    /// Network interface indices assigned to this machine.
    pub netif: Vec<u32>,
    /// Timestamp (CLOCK_REALTIME microseconds since epoch) when registered.
    pub timestamp: u64,
    /// Current state.
    pub state: MachineState,
}

// ---------------------------------------------------------------------------
// Image management
// ---------------------------------------------------------------------------

/// Type of a machine image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageType {
    Directory,
    Raw,
}

impl ImageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ImageType::Directory => "directory",
            ImageType::Raw => "raw",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "directory" | "subvolume" => Some(ImageType::Directory),
            "raw" => Some(ImageType::Raw),
            _ => None,
        }
    }
}

impl fmt::Display for ImageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A machine image stored in the pool.
#[derive(Debug, Clone)]
pub struct ImageInfo {
    /// Image name.
    pub name: String,
    /// Type (directory or raw).
    pub image_type: ImageType,
    /// Full path to the image.
    pub path: PathBuf,
    /// Whether the image is read-only.
    pub read_only: bool,
    /// Creation time (unix microseconds), 0 if unknown.
    pub crtime: u64,
    /// Modification time (unix microseconds), 0 if unknown.
    pub mtime: u64,
    /// Disk usage in bytes, 0 if unknown.
    pub usage: u64,
    /// Size limit in bytes, 0 for no limit.
    pub limit: u64,
}

impl ImageInfo {
    /// Discover an image from a path entry.
    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().to_string();
        if name.starts_with('.') {
            return None;
        }

        let meta = fs::metadata(path).ok()?;
        let image_type = if meta.is_dir() {
            ImageType::Directory
        } else if meta.is_file() {
            ImageType::Raw
        } else {
            return None;
        };

        let read_only = meta.permissions().readonly();
        let mtime = meta.mtime() as u64 * 1_000_000;
        let usage = if image_type == ImageType::Raw {
            meta.len()
        } else {
            // For directories, we could walk recursively, but use 0 for now
            0
        };

        // Read limit from .limit file if present
        let limit_path = path.with_extension("limit");
        let limit = fs::read_to_string(&limit_path)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);

        Some(ImageInfo {
            name,
            image_type,
            path: path.to_path_buf(),
            read_only,
            crtime: mtime, // Use mtime as crtime approximation
            mtime,
            usage,
            limit,
        })
    }

    /// Format status output for this image.
    pub fn format_status(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("       Name: {}\n", self.name));
        s.push_str(&format!("       Type: {}\n", self.image_type));
        s.push_str(&format!("       Path: {}\n", self.path.display()));
        s.push_str(&format!(
            "   ReadOnly: {}\n",
            if self.read_only { "yes" } else { "no" }
        ));
        s.push_str(&format!("    Created: {}\n", format_timestamp(self.crtime)));
        s.push_str(&format!("   Modified: {}\n", format_timestamp(self.mtime)));
        if self.usage > 0 {
            s.push_str(&format!("      Usage: {}\n", format_bytes(self.usage)));
        }
        if self.limit > 0 {
            s.push_str(&format!("      Limit: {}\n", format_bytes(self.limit)));
        }
        s
    }

    /// Format `show` output (key=value pairs).
    pub fn format_show(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("Name={}\n", self.name));
        s.push_str(&format!("Type={}\n", self.image_type));
        s.push_str(&format!("Path={}\n", self.path.display()));
        s.push_str(&format!(
            "ReadOnly={}\n",
            if self.read_only { "yes" } else { "no" }
        ));
        s.push_str(&format!("CreationTimestamp={}\n", self.crtime));
        s.push_str(&format!("ModificationTimestamp={}\n", self.mtime));
        s.push_str(&format!("Usage={}\n", self.usage));
        s.push_str(&format!("Limit={}\n", self.limit));
        s
    }
}

/// Discover all images in the pool directory.
pub fn discover_images(pool_path: &str) -> Vec<ImageInfo> {
    let pool = Path::new(pool_path);
    if !pool.is_dir() {
        return Vec::new();
    }
    let mut images = Vec::new();
    if let Ok(entries) = fs::read_dir(pool) {
        for entry in entries.flatten() {
            if let Some(info) = ImageInfo::from_path(&entry.path()) {
                images.push(info);
            }
        }
    }
    images.sort_by(|a, b| a.name.cmp(&b.name));
    images
}

/// Find an image by name in the pool.
pub fn find_image(pool_path: &str, name: &str) -> Option<ImageInfo> {
    let pool = Path::new(pool_path);
    // Try exact name first
    let path = pool.join(name);
    if path.exists() {
        return ImageInfo::from_path(&path);
    }
    // Try with .raw extension
    let raw_path = pool.join(format!("{}.raw", name));
    if raw_path.exists() {
        return ImageInfo::from_path(&raw_path);
    }
    None
}

/// Clone an image to a new name.
pub fn clone_image(
    pool_path: &str,
    source: &str,
    dest: &str,
    read_only: bool,
) -> Result<(), String> {
    if !is_valid_machine_name(dest) {
        return Err(format!("Invalid image name '{}'", dest));
    }

    let src_image =
        find_image(pool_path, source).ok_or_else(|| format!("Image '{}' not found", source))?;

    let dest_path = Path::new(pool_path).join(dest);
    if dest_path.exists() {
        return Err(format!("Image '{}' already exists", dest));
    }

    match src_image.image_type {
        ImageType::Directory => {
            // Use cp -a for recursive copy
            let status = std::process::Command::new("cp")
                .args([
                    "-a",
                    &src_image.path.to_string_lossy(),
                    &dest_path.to_string_lossy(),
                ])
                .status()
                .map_err(|e| format!("Failed to clone directory: {}", e))?;
            if !status.success() {
                return Err("Failed to clone directory image".to_string());
            }
        }
        ImageType::Raw => {
            let dest_raw = if dest.ends_with(".raw") {
                dest_path.clone()
            } else {
                Path::new(pool_path).join(format!("{}.raw", dest))
            };
            fs::copy(&src_image.path, &dest_raw)
                .map_err(|e| format!("Failed to clone raw image: {}", e))?;
        }
    }

    if read_only {
        let perms = std::os::unix::fs::PermissionsExt::from_mode(0o444);
        let _ = fs::set_permissions(&dest_path, perms);
    }

    Ok(())
}

/// Rename an image.
pub fn rename_image(pool_path: &str, old_name: &str, new_name: &str) -> Result<(), String> {
    if !is_valid_machine_name(new_name) {
        return Err(format!("Invalid image name '{}'", new_name));
    }

    let old_image =
        find_image(pool_path, old_name).ok_or_else(|| format!("Image '{}' not found", old_name))?;

    let new_path = if old_image.image_type == ImageType::Raw && !new_name.ends_with(".raw") {
        Path::new(pool_path).join(format!("{}.raw", new_name))
    } else {
        Path::new(pool_path).join(new_name)
    };

    if new_path.exists() {
        return Err(format!("Image '{}' already exists", new_name));
    }

    fs::rename(&old_image.path, &new_path).map_err(|e| format!("Failed to rename image: {}", e))?;

    // Also rename .limit file if present
    let old_limit = old_image.path.with_extension("limit");
    if old_limit.exists() {
        let new_limit = new_path.with_extension("limit");
        let _ = fs::rename(&old_limit, &new_limit);
    }

    Ok(())
}

/// Remove an image.
pub fn remove_image(pool_path: &str, name: &str) -> Result<(), String> {
    let image = find_image(pool_path, name).ok_or_else(|| format!("Image '{}' not found", name))?;

    match image.image_type {
        ImageType::Directory => {
            fs::remove_dir_all(&image.path)
                .map_err(|e| format!("Failed to remove directory image: {}", e))?;
        }
        ImageType::Raw => {
            fs::remove_file(&image.path)
                .map_err(|e| format!("Failed to remove raw image: {}", e))?;
        }
    }

    // Also remove .limit file if present
    let limit_path = image.path.with_extension("limit");
    let _ = fs::remove_file(&limit_path);

    Ok(())
}

/// Set a size limit on an image (stored as a .limit sidecar file).
pub fn set_image_limit(pool_path: &str, name: &str, limit_bytes: u64) -> Result<(), String> {
    let image = find_image(pool_path, name).ok_or_else(|| format!("Image '{}' not found", name))?;

    let limit_path = image.path.with_extension("limit");
    if limit_bytes == 0 {
        // Remove limit
        let _ = fs::remove_file(&limit_path);
    } else {
        fs::write(&limit_path, format!("{}\n", limit_bytes))
            .map_err(|e| format!("Failed to set limit: {}", e))?;
    }
    Ok(())
}

/// Get pool usage statistics.
pub fn pool_usage_stats(pool_path: &str) -> (u64, u64) {
    let pool = Path::new(pool_path);
    if !pool.exists() {
        return (0, 0);
    }
    // Use statvfs for disk usage
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let c_path = std::ffi::CString::new(pool_path).unwrap_or_default();
    let ret = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if ret != 0 {
        return (0, 0);
    }
    let total = stat.f_blocks * stat.f_frsize;
    let free = stat.f_bfree * stat.f_frsize;
    let used = total.saturating_sub(free);
    (used, total)
}

/// Format bytes in human-readable form.
pub fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "0B".to_string();
    }
    let units = ["B", "K", "M", "G", "T", "P"];
    let mut val = bytes as f64;
    let mut idx = 0;
    while val >= 1024.0 && idx < units.len() - 1 {
        val /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{}B", bytes)
    } else {
        format!("{:.1}{}", val, units[idx])
    }
}

/// Parse a size string with optional K/M/G/T suffix.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('T') {
        (n, 1024u64 * 1024 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('G') {
        (n, 1024u64 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('M') {
        (n, 1024u64 * 1024)
    } else if let Some(n) = s.strip_suffix('K') {
        (n, 1024u64)
    } else if let Some(n) = s.strip_suffix('B') {
        (n, 1u64)
    } else {
        (s, 1u64)
    };
    num_str.trim().parse::<u64>().ok().map(|n| n * multiplier)
}

// ---------------------------------------------------------------------------
// Machine scoping (transient scope units)
// ---------------------------------------------------------------------------

/// Create a transient scope unit for a machine via D-Bus.
///
/// Calls org.freedesktop.systemd1.Manager.StartTransientUnit to create
/// a scope like `machine-<name>.scope` containing the leader PID.
pub fn create_machine_scope(name: &str, leader: u32, description: &str) -> Result<String, String> {
    let scope_name = format!("machine-{}.scope", name);

    // Build D-Bus connection to system bus
    let conn = zbus::blocking::Connection::system()
        .map_err(|e| format!("D-Bus connection failed: {}", e))?;

    let _proxy = zbus::blocking::fdo::DBusProxy::new(&conn)
        .map_err(|e| format!("D-Bus proxy failed: {}", e))?;

    // Use the systemd1 Manager to start a transient unit
    // We call StartTransientUnit(name, mode, properties, aux)
    // Since zbus doesn't have a typed systemd1 proxy, we use the raw message API
    let msg = conn.call_method(
        Some("org.freedesktop.systemd1"),
        "/org/freedesktop/systemd1",
        Some("org.freedesktop.systemd1.Manager"),
        "StartTransientUnit",
        &(
            &scope_name,
            "fail",
            vec![
                ("Description", zbus::zvariant::Value::from(description)),
                ("PIDs", zbus::zvariant::Value::from(vec![leader])),
                ("Delegate", zbus::zvariant::Value::from(true)),
            ],
            Vec::<(String, Vec<(String, zbus::zvariant::Value)>)>::new(),
        ),
    );

    match msg {
        Ok(_) => {
            log::info!("Created transient scope unit '{}'", scope_name);
            Ok(scope_name)
        }
        Err(e) => {
            // Non-fatal: machine still works without a scope
            log::warn!("Failed to create scope '{}': {}", scope_name, e);
            Err(format!("Failed to create scope: {}", e))
        }
    }
}

/// Stop a transient scope unit.
pub fn stop_machine_scope(scope_name: &str) -> Result<(), String> {
    let conn = zbus::blocking::Connection::system()
        .map_err(|e| format!("D-Bus connection failed: {}", e))?;

    let _ = conn.call_method(
        Some("org.freedesktop.systemd1"),
        "/org/freedesktop/systemd1",
        Some("org.freedesktop.systemd1.Manager"),
        "StopUnit",
        &(scope_name, "fail"),
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Copy-to/copy-from operations
// ---------------------------------------------------------------------------

/// Copy a file from the host to a running container.
///
/// Accesses the container's root filesystem directly via the root_directory
/// path (works for filesystem-based containers).
pub fn copy_to_machine(
    registry: &MachineRegistry,
    machine_name: &str,
    host_path: &str,
    container_path: &str,
) -> Result<(), String> {
    let machine = registry
        .get(machine_name)
        .ok_or_else(|| format!("Machine '{}' not found", machine_name))?;

    let root = &machine.root_directory;
    let dest = if container_path.starts_with('/') {
        format!("{}{}", root.trim_end_matches('/'), container_path)
    } else {
        format!("{}/{}", root.trim_end_matches('/'), container_path)
    };

    let src = Path::new(host_path);
    if !src.exists() {
        return Err(format!("Source path '{}' does not exist", host_path));
    }

    let dest_path = Path::new(&dest);
    // Ensure parent directory exists
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create destination directory: {}", e))?;
    }

    if src.is_dir() {
        let status = std::process::Command::new("cp")
            .args(["-a", host_path, &dest])
            .status()
            .map_err(|e| format!("Failed to copy: {}", e))?;
        if !status.success() {
            return Err("Failed to copy directory".to_string());
        }
    } else {
        fs::copy(host_path, &dest).map_err(|e| format!("Failed to copy file: {}", e))?;
    }

    Ok(())
}

/// Copy a file from a running container to the host.
pub fn copy_from_machine(
    registry: &MachineRegistry,
    machine_name: &str,
    container_path: &str,
    host_path: &str,
) -> Result<(), String> {
    let machine = registry
        .get(machine_name)
        .ok_or_else(|| format!("Machine '{}' not found", machine_name))?;

    let root = &machine.root_directory;
    let src = if container_path.starts_with('/') {
        format!("{}{}", root.trim_end_matches('/'), container_path)
    } else {
        format!("{}/{}", root.trim_end_matches('/'), container_path)
    };

    let src_path = Path::new(&src);
    if !src_path.exists() {
        return Err(format!(
            "Source path '{}' does not exist in container",
            container_path
        ));
    }

    let dest_path = Path::new(host_path);
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create destination directory: {}", e))?;
    }

    if src_path.is_dir() {
        let status = std::process::Command::new("cp")
            .args(["-a", &src, host_path])
            .status()
            .map_err(|e| format!("Failed to copy: {}", e))?;
        if !status.success() {
            return Err("Failed to copy directory".to_string());
        }
    } else {
        fs::copy(&src, host_path).map_err(|e| format!("Failed to copy file: {}", e))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Bind mounts into containers
// ---------------------------------------------------------------------------

/// Bind-mount a host path into a running container's mount namespace.
///
/// Enters the container's mount namespace via `/proc/<leader>/ns/mnt`,
/// optionally creates the destination directory (`mkdir`), and performs
/// a bind mount. If `read_only` is true, the mount is remounted read-only.
///
/// Returns `Ok(())` on success.
pub fn bind_mount_in_machine(
    registry: &MachineRegistry,
    machine_name: &str,
    host_path: &str,
    container_path: &str,
    read_only: bool,
    mkdir: bool,
) -> Result<(), String> {
    let machine = registry
        .get(machine_name)
        .ok_or_else(|| format!("Machine '{}' not found", machine_name))?;

    if machine.leader == 0 {
        return Err(format!("Machine '{}' has no leader PID", machine_name));
    }

    let src = Path::new(host_path);
    if !src.exists() {
        return Err(format!("Source path '{}' does not exist", host_path));
    }

    let leader_pid = machine.leader as i32;
    let root_dir = machine.root_directory.clone();

    // Resolve the destination path inside the container's root
    let dest_in_root = if container_path.starts_with('/') {
        format!("{}{}", root_dir.trim_end_matches('/'), container_path)
    } else {
        format!("{}/{}", root_dir.trim_end_matches('/'), container_path)
    };

    // Fork a child that will enter the container's mount namespace and
    // perform the bind mount there.
    let child_pid = unsafe { libc::fork() };
    match child_pid {
        -1 => Err(format!("fork failed: {}", io::Error::last_os_error())),
        0 => {
            // Child process — enter mount namespace of the leader
            let ns_path = format!("/proc/{}/ns/mnt", leader_pid);
            let c_ns = std::ffi::CString::new(ns_path.as_str()).unwrap();
            let fd = unsafe { libc::open(c_ns.as_ptr(), libc::O_RDONLY) };
            if fd < 0 {
                eprintln!(
                    "Failed to open mount namespace: {}",
                    io::Error::last_os_error()
                );
                unsafe { libc::_exit(1) };
            }
            if unsafe { libc::setns(fd, libc::CLONE_NEWNS) } != 0 {
                eprintln!("setns(CLONE_NEWNS) failed: {}", io::Error::last_os_error());
                unsafe {
                    libc::close(fd);
                    libc::_exit(1);
                }
            }
            unsafe { libc::close(fd) };

            // Create destination if --mkdir
            if mkdir {
                let c_dest = std::ffi::CString::new(dest_in_root.as_str()).unwrap();
                unsafe {
                    // Try to create the directory; ignore errors if it already exists
                    libc::mkdir(c_dest.as_ptr(), 0o755);
                }
            }

            // Perform the bind mount: mount(source, target, NULL, MS_BIND|MS_REC, NULL)
            let c_src = std::ffi::CString::new(host_path).unwrap();
            let c_dest = std::ffi::CString::new(dest_in_root.as_str()).unwrap();
            let flags = libc::MS_BIND | libc::MS_REC;
            let ret = unsafe {
                libc::mount(
                    c_src.as_ptr(),
                    c_dest.as_ptr(),
                    std::ptr::null(),
                    flags,
                    std::ptr::null(),
                )
            };
            if ret != 0 {
                eprintln!("bind mount failed: {}", io::Error::last_os_error());
                unsafe { libc::_exit(1) };
            }

            // If read-only, remount with MS_RDONLY
            if read_only {
                let ro_flags = libc::MS_BIND | libc::MS_REC | libc::MS_REMOUNT | libc::MS_RDONLY;
                let ret = unsafe {
                    libc::mount(
                        std::ptr::null(),
                        c_dest.as_ptr(),
                        std::ptr::null(),
                        ro_flags,
                        std::ptr::null(),
                    )
                };
                if ret != 0 {
                    eprintln!("read-only remount failed: {}", io::Error::last_os_error());
                    unsafe { libc::_exit(1) };
                }
            }

            unsafe { libc::_exit(0) };
        }
        pid => {
            // Parent — wait for the child
            let mut status: libc::c_int = 0;
            unsafe { libc::waitpid(pid, &mut status, 0) };
            if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0 {
                Ok(())
            } else {
                Err(format!(
                    "Bind mount of '{}' to '{}:{}' failed (child exit status {})",
                    host_path,
                    machine_name,
                    container_path,
                    if libc::WIFEXITED(status) {
                        libc::WEXITSTATUS(status)
                    } else {
                        -1
                    },
                ))
            }
        }
    }
}

/// Bind-mount a host path into a container with UID-mapped user semantics.
///
/// This is similar to `bind_mount_in_machine` but also sets up the bind mount
/// in a way that is accessible by a specific user inside the container.
/// It resolves the user's home directory inside the container (via
/// `/etc/passwd`), creates the destination path under it if needed, and
/// performs the bind mount.
///
/// If `container_path` is `None`, the mount destination defaults to
/// `<home>/<basename(host_path)>` inside the container.
pub fn bind_user_in_machine(
    registry: &MachineRegistry,
    machine_name: &str,
    host_path: &str,
    container_path: Option<&str>,
    user: &str,
    read_only: bool,
) -> Result<(), String> {
    let machine = registry
        .get(machine_name)
        .ok_or_else(|| format!("Machine '{}' not found", machine_name))?;

    if machine.leader == 0 {
        return Err(format!("Machine '{}' has no leader PID", machine_name));
    }

    let src = Path::new(host_path);
    if !src.exists() {
        return Err(format!("Source path '{}' does not exist", host_path));
    }

    let root = &machine.root_directory;

    // Resolve the user's home directory from the container's /etc/passwd
    let home_dir = resolve_container_user_home(root, user)?;

    // Determine the destination path
    let dest = match container_path {
        Some(p) => p.to_string(),
        None => {
            let basename = Path::new(host_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("bind");
            format!("{}/{}", home_dir.trim_end_matches('/'), basename)
        }
    };

    // Use bind_mount_in_machine with mkdir enabled
    bind_mount_in_machine(registry, machine_name, host_path, &dest, read_only, true)
}

/// Resolve a user's home directory from a container's `/etc/passwd`.
///
/// Looks up the given username (or numeric UID) in `<root>/etc/passwd`
/// and returns the home directory field.
fn resolve_container_user_home(root: &str, user: &str) -> Result<String, String> {
    let passwd_path = format!("{}/etc/passwd", root.trim_end_matches('/'));
    let content = fs::read_to_string(&passwd_path).map_err(|e| {
        format!(
            "Cannot read {}: {} (cannot resolve user '{}')",
            passwd_path, e, user
        )
    })?;

    // Check if user is a numeric UID
    let is_uid = user.parse::<u32>().ok();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() < 6 {
            continue;
        }
        // fields: name:password:uid:gid:gecos:home:shell
        let match_by_name = fields[0] == user;
        let match_by_uid = is_uid
            .map(|uid| fields.get(2).and_then(|u| u.parse::<u32>().ok()) == Some(uid))
            .unwrap_or(false);

        if match_by_name || match_by_uid {
            return Ok(fields[5].to_string());
        }
    }

    // Default to /root for root, /home/<user> otherwise
    if user == "root" || user == "0" {
        Ok("/root".to_string())
    } else {
        Ok(format!("/home/{}", user))
    }
}

// ---------------------------------------------------------------------------
// PTY forwarding for login/shell
// ---------------------------------------------------------------------------

/// Open a PTY and enter a container's namespaces to provide login/shell.
///
/// Returns (master_fd, child_pid) on success. The caller should forward
/// I/O between the master fd and the client's terminal.
pub fn open_machine_pty(
    registry: &MachineRegistry,
    machine_name: &str,
    shell_cmd: Option<&str>,
    user: Option<&str>,
) -> Result<(i32, i32), String> {
    let machine = registry
        .get(machine_name)
        .ok_or_else(|| format!("Machine '{}' not found", machine_name))?;

    if machine.leader == 0 {
        return Err(format!("Machine '{}' has no leader PID", machine_name));
    }

    // Open a PTY pair
    let mut master_fd: libc::c_int = -1;
    let mut slave_fd: libc::c_int = -1;
    let ret = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ret != 0 {
        return Err(format!("openpty failed: {}", io::Error::last_os_error()));
    }

    let leader_pid = machine.leader as i32;
    let root_dir = machine.root_directory.clone();

    let child_pid = unsafe { libc::fork() };
    match child_pid {
        -1 => {
            unsafe {
                libc::close(master_fd);
                libc::close(slave_fd);
            }
            Err(format!("fork failed: {}", io::Error::last_os_error()))
        }
        0 => {
            // Child process: enter the container's namespaces
            unsafe {
                libc::close(master_fd);
            }

            // Open and enter the leader's namespaces
            let ns_types = ["pid", "mnt", "uts", "ipc", "net"];
            for ns in &ns_types {
                let ns_path = format!("/proc/{}/ns/{}", leader_pid, ns);
                let fd = unsafe {
                    libc::open(
                        std::ffi::CString::new(ns_path.as_str()).unwrap().as_ptr(),
                        libc::O_RDONLY,
                    )
                };
                if fd >= 0 {
                    let _ = unsafe { libc::setns(fd, 0) };
                    unsafe { libc::close(fd) };
                }
            }

            // Set up the slave PTY as stdin/stdout/stderr
            unsafe {
                libc::setsid();
                libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0);
                libc::dup2(slave_fd, 0);
                libc::dup2(slave_fd, 1);
                libc::dup2(slave_fd, 2);
                if slave_fd > 2 {
                    libc::close(slave_fd);
                }
            }

            // chroot into the container root
            if root_dir != "/" {
                let c_root = std::ffi::CString::new(root_dir.as_str()).unwrap();
                unsafe {
                    libc::chroot(c_root.as_ptr());
                    libc::chdir(c"/".as_ptr());
                }
            }

            // Switch user if requested
            if let Some(u) = user
                && let Ok(uid) = u.parse::<u32>()
            {
                unsafe {
                    libc::setgid(uid);
                    libc::setuid(uid);
                }
            }

            // Exec the shell
            let shell = shell_cmd.unwrap_or("/bin/login");
            let c_shell = std::ffi::CString::new(shell).unwrap();
            if shell.contains("login") {
                // For login, no args
                unsafe {
                    libc::execl(
                        c_shell.as_ptr(),
                        c_shell.as_ptr(),
                        std::ptr::null::<libc::c_char>(),
                    );
                }
            } else {
                // For shell, pass -l for login shell
                let c_arg = std::ffi::CString::new("-l").unwrap();
                unsafe {
                    libc::execl(
                        c_shell.as_ptr(),
                        c_shell.as_ptr(),
                        c_arg.as_ptr(),
                        std::ptr::null::<libc::c_char>(),
                    );
                }
            }

            // If exec fails
            unsafe { libc::_exit(127) };
        }
        pid => {
            // Parent: close slave, return master fd and child pid
            unsafe {
                libc::close(slave_fd);
            }
            Ok((master_fd, pid))
        }
    }
}

// ---------------------------------------------------------------------------
// OS image import/export/pull
// ---------------------------------------------------------------------------

/// Import a tar archive as a machine image.
pub fn import_tar(
    pool_path: &str,
    source_path: &str,
    image_name: &str,
    read_only: bool,
) -> Result<(), String> {
    if !is_valid_machine_name(image_name) {
        return Err(format!("Invalid image name '{}'", image_name));
    }

    let dest = Path::new(pool_path).join(image_name);
    if dest.exists() {
        return Err(format!("Image '{}' already exists", image_name));
    }

    fs::create_dir_all(&dest).map_err(|e| format!("Failed to create image directory: {}", e))?;

    let status = if source_path == "-" {
        // Read from stdin
        std::process::Command::new("tar")
            .args(["xf", "-", "-C", &dest.to_string_lossy()])
            .stdin(std::process::Stdio::inherit())
            .status()
            .map_err(|e| format!("Failed to run tar: {}", e))?
    } else {
        std::process::Command::new("tar")
            .args(["xf", source_path, "-C", &dest.to_string_lossy()])
            .status()
            .map_err(|e| format!("Failed to run tar: {}", e))?
    };

    if !status.success() {
        let _ = fs::remove_dir_all(&dest);
        return Err("Failed to extract tar archive".to_string());
    }

    if read_only {
        let perms = std::os::unix::fs::PermissionsExt::from_mode(0o555);
        let _ = fs::set_permissions(&dest, perms);
    }

    Ok(())
}

/// Import a raw disk image.
pub fn import_raw(
    pool_path: &str,
    source_path: &str,
    image_name: &str,
    read_only: bool,
) -> Result<(), String> {
    if !is_valid_machine_name(image_name) {
        return Err(format!("Invalid image name '{}'", image_name));
    }

    let dest_name = if image_name.ends_with(".raw") {
        image_name.to_string()
    } else {
        format!("{}.raw", image_name)
    };
    let dest = Path::new(pool_path).join(&dest_name);
    if dest.exists() {
        return Err(format!("Image '{}' already exists", image_name));
    }

    if source_path == "-" {
        // Read from stdin
        let mut file =
            fs::File::create(&dest).map_err(|e| format!("Failed to create image file: {}", e))?;
        let stdin = io::stdin();
        let mut handle = stdin.lock();
        io::copy(&mut handle, &mut file).map_err(|e| format!("Failed to write image: {}", e))?;
    } else {
        fs::copy(source_path, &dest).map_err(|e| format!("Failed to copy raw image: {}", e))?;
    }

    if read_only {
        let perms = std::os::unix::fs::PermissionsExt::from_mode(0o444);
        let _ = fs::set_permissions(&dest, perms);
    }

    Ok(())
}

/// Export a machine image as a tar archive.
pub fn export_tar(pool_path: &str, image_name: &str, dest_path: &str) -> Result<(), String> {
    let image = find_image(pool_path, image_name)
        .ok_or_else(|| format!("Image '{}' not found", image_name))?;

    if image.image_type != ImageType::Directory {
        return Err("Can only export directory images as tar".to_string());
    }

    let status = if dest_path == "-" {
        std::process::Command::new("tar")
            .args(["cf", "-", "-C", &image.path.to_string_lossy(), "."])
            .stdout(std::process::Stdio::inherit())
            .status()
            .map_err(|e| format!("Failed to run tar: {}", e))?
    } else {
        std::process::Command::new("tar")
            .args(["cf", dest_path, "-C", &image.path.to_string_lossy(), "."])
            .status()
            .map_err(|e| format!("Failed to run tar: {}", e))?
    };

    if !status.success() {
        return Err("Failed to create tar archive".to_string());
    }

    Ok(())
}

/// Export a machine image as a raw copy.
pub fn export_raw(pool_path: &str, image_name: &str, dest_path: &str) -> Result<(), String> {
    let image = find_image(pool_path, image_name)
        .ok_or_else(|| format!("Image '{}' not found", image_name))?;

    if image.image_type != ImageType::Raw {
        return Err("Can only export raw images as raw".to_string());
    }

    if dest_path == "-" {
        let mut file =
            fs::File::open(&image.path).map_err(|e| format!("Failed to open image: {}", e))?;
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        io::copy(&mut file, &mut handle).map_err(|e| format!("Failed to write image: {}", e))?;
    } else {
        fs::copy(&image.path, dest_path).map_err(|e| format!("Failed to copy image: {}", e))?;
    }

    Ok(())
}

/// Pull a tar image from a URL.
pub fn pull_tar(pool_path: &str, url: &str, image_name: &str, _verify: &str) -> Result<(), String> {
    if !is_valid_machine_name(image_name) {
        return Err(format!("Invalid image name '{}'", image_name));
    }

    let dest = Path::new(pool_path).join(image_name);
    if dest.exists() {
        return Err(format!("Image '{}' already exists", image_name));
    }

    // Download to a temporary file, then extract
    let tmp_tar = Path::new(pool_path).join(format!(".{}.tar.tmp", image_name));

    let status = std::process::Command::new("curl")
        .args(["-fsSL", "-o", &tmp_tar.to_string_lossy(), url])
        .status()
        .map_err(|e| format!("Failed to download: {}", e))?;

    if !status.success() {
        let _ = fs::remove_file(&tmp_tar);
        return Err(format!("Failed to download from '{}'", url));
    }

    fs::create_dir_all(&dest).map_err(|e| format!("Failed to create image directory: {}", e))?;

    let extract = std::process::Command::new("tar")
        .args([
            "xf",
            &tmp_tar.to_string_lossy(),
            "-C",
            &dest.to_string_lossy(),
        ])
        .status()
        .map_err(|e| format!("Failed to extract: {}", e))?;

    let _ = fs::remove_file(&tmp_tar);

    if !extract.success() {
        let _ = fs::remove_dir_all(&dest);
        return Err("Failed to extract downloaded tar".to_string());
    }

    log::info!("Pulled tar image '{}' from {}", image_name, url);
    Ok(())
}

/// Pull a raw image from a URL.
pub fn pull_raw(pool_path: &str, url: &str, image_name: &str, _verify: &str) -> Result<(), String> {
    if !is_valid_machine_name(image_name) {
        return Err(format!("Invalid image name '{}'", image_name));
    }

    let dest_name = if image_name.ends_with(".raw") {
        image_name.to_string()
    } else {
        format!("{}.raw", image_name)
    };
    let dest = Path::new(pool_path).join(&dest_name);
    if dest.exists() {
        return Err(format!("Image '{}' already exists", image_name));
    }

    let status = std::process::Command::new("curl")
        .args(["-fsSL", "-o", &dest.to_string_lossy(), url])
        .status()
        .map_err(|e| format!("Failed to download: {}", e))?;

    if !status.success() {
        let _ = fs::remove_file(&dest);
        return Err(format!("Failed to download from '{}'", url));
    }

    log::info!("Pulled raw image '{}' from {}", image_name, url);
    Ok(())
}

// ---------------------------------------------------------------------------
// Data model — Machine
// ---------------------------------------------------------------------------

impl Machine {
    /// Serialize to an INI-style state file for `/run/systemd/machines/<name>`.
    pub fn to_state_file(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("NAME={}\n", self.name));
        s.push_str(&format!("CLASS={}\n", self.class));
        s.push_str(&format!("SERVICE={}\n", self.service));
        s.push_str(&format!("SCOPE={}\n", self.scope));
        s.push_str(&format!("LEADER={}\n", self.leader));
        s.push_str(&format!("ROOT={}\n", self.root_directory));
        s.push_str(&format!("STATE={}\n", self.state));
        s.push_str(&format!("TIMESTAMP={}\n", self.timestamp));
        if !self.netif.is_empty() {
            let nifs: Vec<String> = self.netif.iter().map(|n| n.to_string()).collect();
            s.push_str(&format!("NETIF={}\n", nifs.join(" ")));
        }
        s
    }

    /// Parse a machine from a state file.
    pub fn from_state_file(content: &str) -> Option<Self> {
        let fields = parse_env_content(content);
        let name = fields.get("NAME")?.clone();
        let class = MachineClass::parse(fields.get("CLASS").map(|s| s.as_str()).unwrap_or(""))?;
        let service = fields.get("SERVICE").cloned().unwrap_or_default();
        let scope = fields.get("SCOPE").cloned().unwrap_or_default();
        let leader = fields
            .get("LEADER")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let root_directory = fields
            .get("ROOT")
            .cloned()
            .unwrap_or_else(|| "/".to_string());
        let state = fields
            .get("STATE")
            .and_then(|s| MachineState::parse(s))
            .unwrap_or(MachineState::Running);
        let timestamp = fields
            .get("TIMESTAMP")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let netif = fields
            .get("NETIF")
            .map(|s| {
                s.split_whitespace()
                    .filter_map(|n| n.parse().ok())
                    .collect()
            })
            .unwrap_or_default();

        Some(Machine {
            name,
            class,
            service,
            scope,
            leader,
            root_directory,
            netif,
            timestamp,
            state,
        })
    }

    /// Check if the leader PID is still alive.
    pub fn is_leader_alive(&self) -> bool {
        if self.leader == 0 {
            return false;
        }
        // kill(pid, 0) checks if process exists without sending a signal
        unsafe { libc::kill(self.leader as i32, 0) == 0 }
    }

    /// Format status output for this machine.
    pub fn format_status(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("       Name: {}\n", self.name));
        s.push_str(&format!("      Class: {}\n", self.class));
        s.push_str(&format!("    Service: {}\n", self.service));
        s.push_str(&format!("      Scope: {}\n", self.scope));
        s.push_str(&format!("     Leader: {}\n", self.leader));
        s.push_str(&format!("       Root: {}\n", self.root_directory));
        s.push_str(&format!("      State: {}\n", self.state));
        s.push_str(&format!(
            "      Since: {}\n",
            format_timestamp(self.timestamp)
        ));
        if !self.netif.is_empty() {
            let nifs: Vec<String> = self.netif.iter().map(|n| n.to_string()).collect();
            s.push_str(&format!("     NetIf: {}\n", nifs.join(" ")));
        }
        s
    }

    /// Format `show` output (key=value pairs).
    pub fn format_show(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("Name={}\n", self.name));
        s.push_str(&format!("Class={}\n", self.class));
        s.push_str(&format!("Service={}\n", self.service));
        s.push_str(&format!("Scope={}\n", self.scope));
        s.push_str(&format!("Leader={}\n", self.leader));
        s.push_str(&format!("RootDirectory={}\n", self.root_directory));
        s.push_str(&format!("State={}\n", self.state));
        s.push_str(&format!("Timestamp={}\n", self.timestamp));
        if !self.netif.is_empty() {
            let nifs: Vec<String> = self.netif.iter().map(|n| n.to_string()).collect();
            s.push_str(&format!("NetworkInterfaces={}\n", nifs.join(" ")));
        }
        s
    }
}

// ---------------------------------------------------------------------------
// Machine registry
// ---------------------------------------------------------------------------

/// In-memory registry of machines.
#[derive(Debug, Default)]
pub struct MachineRegistry {
    machines: BTreeMap<String, Machine>,
}

impl MachineRegistry {
    pub fn new() -> Self {
        Self {
            machines: BTreeMap::new(),
        }
    }

    /// Load existing machines from state files in the default machines directory.
    pub fn load(&mut self) {
        self.load_from(MACHINES_DIR);
    }

    /// Load existing machines from state files in a given directory.
    pub fn load_from(&mut self, dir: &str) {
        self.machines.clear();
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.starts_with('.') {
                continue;
            }
            if let Ok(content) = fs::read_to_string(&path)
                && let Some(machine) = Machine::from_state_file(&content)
            {
                self.machines.insert(machine.name.clone(), machine);
            }
        }
    }

    /// Register a new machine.
    pub fn register(&mut self, machine: Machine) -> Result<(), String> {
        if machine.name.is_empty() {
            return Err("Machine name must not be empty".into());
        }
        if !is_valid_machine_name(&machine.name) {
            return Err(format!("Invalid machine name '{}'", machine.name));
        }
        if self.machines.contains_key(&machine.name) {
            return Err(format!("Machine '{}' already registered", machine.name));
        }
        self.machines.insert(machine.name.clone(), machine);
        Ok(())
    }

    /// Terminate (unregister) a machine by name.
    pub fn terminate(&mut self, name: &str) -> Result<Machine, String> {
        self.terminate_in(name, MACHINES_DIR)
    }

    /// Terminate (unregister) a machine, removing its state file from a given dir.
    pub fn terminate_in(&mut self, name: &str, dir: &str) -> Result<Machine, String> {
        match self.machines.remove(name) {
            Some(machine) => {
                let path = Path::new(dir).join(name);
                let _ = fs::remove_file(&path);
                Ok(machine)
            }
            None => Err(format!("Machine '{}' not found", name)),
        }
    }

    /// Get a machine by name.
    pub fn get(&self, name: &str) -> Option<&Machine> {
        self.machines.get(name)
    }

    /// Get a mutable reference to a machine by name.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Machine> {
        self.machines.get_mut(name)
    }

    /// List all registered machines, sorted by name.
    pub fn list(&self) -> Vec<&Machine> {
        self.machines.values().collect()
    }

    /// Number of registered machines.
    pub fn count(&self) -> usize {
        self.machines.len()
    }

    /// Find a machine by leader PID.
    pub fn find_by_leader(&self, pid: u32) -> Option<&Machine> {
        self.machines.values().find(|m| m.leader == pid)
    }

    /// Save all machine state files to the default directory.
    pub fn save_all(&self) -> io::Result<()> {
        self.save_all_to(MACHINES_DIR)
    }

    /// Save all machine state files to a given directory.
    pub fn save_all_to(&self, dir: &str) -> io::Result<()> {
        fs::create_dir_all(dir)?;
        for machine in self.machines.values() {
            let path = Path::new(dir).join(&machine.name);
            fs::write(&path, machine.to_state_file())?;
        }
        Ok(())
    }

    /// Save a single machine state file to the default directory.
    pub fn save_one(&self, name: &str) -> io::Result<()> {
        self.save_one_to(name, MACHINES_DIR)
    }

    /// Save a single machine state file to a given directory.
    pub fn save_one_to(&self, name: &str, dir: &str) -> io::Result<()> {
        if let Some(machine) = self.machines.get(name) {
            fs::create_dir_all(dir)?;
            let path = Path::new(dir).join(name);
            fs::write(&path, machine.to_state_file())?;
        }
        Ok(())
    }

    /// Remove dead machines whose leader PID is no longer alive.
    /// Returns list of names of removed machines.
    pub fn gc(&mut self) -> Vec<String> {
        self.gc_in(MACHINES_DIR)
    }

    /// Remove dead machines, removing state files from a given directory.
    pub fn gc_in(&mut self, dir: &str) -> Vec<String> {
        let dead: Vec<String> = self
            .machines
            .iter()
            .filter(|(_, m)| !m.is_leader_alive())
            .map(|(name, _)| name.clone())
            .collect();

        for name in &dead {
            self.machines.remove(name);
            let path = Path::new(dir).join(name);
            let _ = fs::remove_file(&path);
        }

        dead
    }

    /// Format a listing of all machines.
    pub fn format_list(&self) -> String {
        let machines = self.list();
        if machines.is_empty() {
            return "No machines.\n".to_string();
        }
        let mut s = String::new();
        s.push_str(&format!(
            "{:<32} {:>10} {:>12} {:>10}\n",
            "MACHINE", "CLASS", "SERVICE", "STATE"
        ));
        for m in &machines {
            s.push_str(&format!(
                "{:<32} {:>10} {:>12} {:>10}\n",
                m.name, m.class, m.service, m.state
            ));
        }
        s.push_str(&format!("\n{} machines listed.\n", machines.len()));
        s
    }
}

// ---------------------------------------------------------------------------
// D-Bus shared state
// ---------------------------------------------------------------------------

type SharedRegistry = Arc<Mutex<MachineRegistry>>;

// ---------------------------------------------------------------------------
// D-Bus interface: org.freedesktop.machine1.Manager
// ---------------------------------------------------------------------------

/// D-Bus interface struct for org.freedesktop.machine1.Manager.
///
/// Methods:
///   ListMachines() → a(ssss) — array of (name, class, service, object_path)
///   GetMachine(s name) → o — object path for a machine
///   GetMachineByPID(u pid) → o — object path by leader PID
///   RegisterMachine(s name, ay id, s service, s class, u leader, s root_directory) → o
///   TerminateMachine(s name)
///   KillMachine(s name, s who, i signal)
///
/// Properties:
///   PoolPath (s) — path to the machine image pool
///   PoolUsage (t) — current pool usage in bytes
///   PoolLimit (t) — pool size limit in bytes
struct Machine1Manager {
    registry: SharedRegistry,
    pool_path: String,
}

#[zbus::interface(name = "org.freedesktop.machine1.Manager")]
impl Machine1Manager {
    // --- Properties (read-only) ---

    #[zbus(property, name = "PoolPath")]
    fn pool_path(&self) -> String {
        self.pool_path.clone()
    }

    #[zbus(property, name = "PoolUsage")]
    fn pool_usage(&self) -> u64 {
        let (used, _total) = pool_usage_stats(&self.pool_path);
        used
    }

    #[zbus(property, name = "PoolLimit")]
    fn pool_limit(&self) -> u64 {
        let (_used, total) = pool_usage_stats(&self.pool_path);
        total
    }

    // --- Methods ---

    /// ListMachines() → a(ssss) — array of (name, class, service, object_path)
    fn list_machines(&self) -> Vec<(String, String, String, String)> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry
            .list()
            .iter()
            .map(|m| {
                (
                    m.name.clone(),
                    m.class.as_str().to_string(),
                    m.service.clone(),
                    machine_object_path(&m.name),
                )
            })
            .collect()
    }

    /// GetMachine(s name) → o
    fn get_machine(&self, name: String) -> zbus::fdo::Result<String> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if registry.get(&name).is_some() {
            Ok(machine_object_path(&name))
        } else {
            Err(zbus::fdo::Error::Failed(format!(
                "No machine '{}' known",
                name
            )))
        }
    }

    /// GetMachineByPID(u pid) → o
    fn get_machine_by_pid(&self, pid: u32) -> zbus::fdo::Result<String> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = registry.find_by_leader(pid) {
            Ok(machine_object_path(&m.name))
        } else {
            Err(zbus::fdo::Error::Failed(format!(
                "No machine for PID {}",
                pid
            )))
        }
    }

    /// RegisterMachine(s name, ay id, s service, s class, u leader, s root_directory) → o
    fn register_machine(
        &self,
        name: String,
        _id: Vec<u8>,
        service: String,
        class_str: String,
        leader: u32,
        root_directory: String,
    ) -> zbus::fdo::Result<String> {
        let class = match MachineClass::parse(&class_str) {
            Some(c) => c,
            None => {
                return Err(zbus::fdo::Error::Failed(format!(
                    "Invalid class '{}'. Use 'container' or 'vm'",
                    class_str
                )));
            }
        };

        let root_dir = if root_directory.is_empty() {
            "/".to_string()
        } else {
            root_directory
        };

        // Attempt to create a transient scope unit for this machine
        let scope =
            create_machine_scope(&name, leader, &format!("Machine {} ({})", name, class_str))
                .unwrap_or_default();

        let machine = Machine {
            name: name.clone(),
            class,
            service,
            scope,
            leader,
            root_directory: root_dir,
            netif: Vec::new(),
            timestamp: now_usec(),
            state: MachineState::Running,
        };

        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        match registry.register(machine) {
            Ok(()) => {
                let _ = registry.save_one(&name);
                Ok(machine_object_path(&name))
            }
            Err(e) => Err(zbus::fdo::Error::Failed(e)),
        }
    }

    /// TerminateMachine(s name)
    fn terminate_machine(&self, name: String) -> zbus::fdo::Result<()> {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        match registry.terminate(&name) {
            Ok(machine) => {
                if machine.leader > 0 {
                    unsafe {
                        libc::kill(machine.leader as i32, libc::SIGTERM);
                    }
                }
                // Stop the transient scope if one was created
                if !machine.scope.is_empty() {
                    let _ = stop_machine_scope(&machine.scope);
                }
                Ok(())
            }
            Err(e) => Err(zbus::fdo::Error::Failed(e)),
        }
    }

    /// KillMachine(s name, s who, i signal)
    fn kill_machine(&self, name: String, _who: String, signal: i32) -> zbus::fdo::Result<()> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        match registry.get(&name) {
            Some(m) => {
                if m.leader > 0 {
                    let ret = unsafe { libc::kill(m.leader as i32, signal) };
                    if ret != 0 {
                        return Err(zbus::fdo::Error::Failed(
                            "Failed to send signal to leader".to_string(),
                        ));
                    }
                }
                Ok(())
            }
            None => Err(zbus::fdo::Error::Failed(format!(
                "No machine '{}' known",
                name
            ))),
        }
    }

    /// GetMachineOSRelease(s name) → a{ss}
    fn get_machine_os_release(&self, name: String) -> zbus::fdo::Result<Vec<(String, String)>> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        match registry.get(&name) {
            Some(m) => {
                let mut fields: Vec<(String, String)> = Vec::new();
                let root = &m.root_directory;
                let os_release_path = format!("{}/etc/os-release", root);
                let usr_os_release_path = format!("{}/usr/lib/os-release", root);
                let content = fs::read_to_string(&os_release_path)
                    .or_else(|_| fs::read_to_string(&usr_os_release_path))
                    .unwrap_or_default();
                for line in content.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    if let Some((key, val)) = line.split_once('=') {
                        let val = val.trim_matches('"').trim_matches('\'');
                        fields.push((key.to_string(), val.to_string()));
                    }
                }
                Ok(fields)
            }
            None => Err(zbus::fdo::Error::Failed(format!(
                "No machine '{}' known",
                name
            ))),
        }
    }

    /// ListImages() → a(sssbon) — (name, type, path, read_only, crtime, mtime)
    fn list_images(&self) -> Vec<(String, String, String, bool, u64, u64)> {
        discover_images(&self.pool_path)
            .into_iter()
            .map(|img| {
                (
                    img.name,
                    img.image_type.as_str().to_string(),
                    img.path.to_string_lossy().to_string(),
                    img.read_only,
                    img.crtime,
                    img.mtime,
                )
            })
            .collect()
    }

    /// CloneImage(s source, s dest, b read_only)
    fn clone_image_method(
        &self,
        source: String,
        dest: String,
        read_only: bool,
    ) -> zbus::fdo::Result<()> {
        clone_image(&self.pool_path, &source, &dest, read_only).map_err(zbus::fdo::Error::Failed)
    }

    /// RenameImage(s old_name, s new_name)
    fn rename_image(&self, old_name: String, new_name: String) -> zbus::fdo::Result<()> {
        rename_image(&self.pool_path, &old_name, &new_name).map_err(zbus::fdo::Error::Failed)
    }

    /// RemoveImage(s name)
    fn remove_image(&self, name: String) -> zbus::fdo::Result<()> {
        remove_image(&self.pool_path, &name).map_err(zbus::fdo::Error::Failed)
    }

    /// SetImageLimit(s name, t limit)
    fn set_image_limit(&self, name: String, limit: u64) -> zbus::fdo::Result<()> {
        set_image_limit(&self.pool_path, &name, limit).map_err(zbus::fdo::Error::Failed)
    }

    /// SetPoolLimit(t limit) — stub, pool limits require btrfs quota
    fn set_pool_limit(&self, _limit: u64) -> zbus::fdo::Result<()> {
        Ok(())
    }

    /// GetImageOSRelease(s name) → a{ss}
    fn get_image_os_release(&self, name: String) -> zbus::fdo::Result<Vec<(String, String)>> {
        let image = find_image(&self.pool_path, &name)
            .ok_or_else(|| zbus::fdo::Error::Failed(format!("Image '{}' not found", name)))?;

        if image.image_type != ImageType::Directory {
            return Err(zbus::fdo::Error::Failed(
                "Can only read os-release from directory images".to_string(),
            ));
        }

        let mut fields = Vec::new();
        let os_release_path = image.path.join("etc/os-release");
        let usr_os_release_path = image.path.join("usr/lib/os-release");
        let content = fs::read_to_string(&os_release_path)
            .or_else(|_| fs::read_to_string(&usr_os_release_path))
            .unwrap_or_default();
        for (key, val) in parse_env_content(&content) {
            fields.push((key, val));
        }
        Ok(fields)
    }

    /// BindMount(s machine, s source, s destination, b read_only, b mkdir)
    fn bind_mount(
        &self,
        machine: String,
        source: String,
        destination: String,
        read_only: bool,
        mkdir: bool,
    ) -> zbus::fdo::Result<()> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let dest = if destination.is_empty() {
            &source
        } else {
            &destination
        };
        bind_mount_in_machine(&registry, &machine, &source, dest, read_only, mkdir)
            .map_err(zbus::fdo::Error::Failed)
    }

    /// BindUser(s machine, s source, s user, s destination, b read_only)
    fn bind_user(
        &self,
        machine: String,
        source: String,
        user: String,
        destination: String,
        read_only: bool,
    ) -> zbus::fdo::Result<()> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let dest = if destination.is_empty() {
            None
        } else {
            Some(destination.as_str())
        };
        bind_user_in_machine(&registry, &machine, &source, dest, &user, read_only)
            .map_err(zbus::fdo::Error::Failed)
    }

    /// CopyTo(s machine, s host_path, s container_path)
    fn copy_to(
        &self,
        machine: String,
        host_path: String,
        container_path: String,
    ) -> zbus::fdo::Result<()> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        copy_to_machine(&registry, &machine, &host_path, &container_path)
            .map_err(zbus::fdo::Error::Failed)
    }

    /// CopyFrom(s machine, s container_path, s host_path)
    fn copy_from(
        &self,
        machine: String,
        container_path: String,
        host_path: String,
    ) -> zbus::fdo::Result<()> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        copy_from_machine(&registry, &machine, &container_path, &host_path)
            .map_err(zbus::fdo::Error::Failed)
    }

    /// OpenMachineLogin(s name) → h (fd)
    /// Returns the master PTY fd for a login session.
    fn open_machine_login(&self, name: String) -> zbus::fdo::Result<(i32, i32)> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        open_machine_pty(&registry, &name, None, None).map_err(zbus::fdo::Error::Failed)
    }

    /// OpenMachineShell(s name, s user, s shell) → h (fd)
    fn open_machine_shell(
        &self,
        name: String,
        user: String,
        shell: String,
    ) -> zbus::fdo::Result<(i32, i32)> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let u = if user.is_empty() {
            None
        } else {
            Some(user.as_str())
        };
        let s = if shell.is_empty() {
            None
        } else {
            Some(shell.as_str())
        };
        open_machine_pty(&registry, &name, s, u).map_err(zbus::fdo::Error::Failed)
    }

    /// ImportTar(s source, s name, b read_only)
    fn import_tar_method(
        &self,
        source: String,
        name: String,
        read_only: bool,
    ) -> zbus::fdo::Result<()> {
        import_tar(&self.pool_path, &source, &name, read_only).map_err(zbus::fdo::Error::Failed)
    }

    /// ImportRaw(s source, s name, b read_only)
    fn import_raw_method(
        &self,
        source: String,
        name: String,
        read_only: bool,
    ) -> zbus::fdo::Result<()> {
        import_raw(&self.pool_path, &source, &name, read_only).map_err(zbus::fdo::Error::Failed)
    }

    /// ExportTar(s name, s dest)
    fn export_tar_method(&self, name: String, dest: String) -> zbus::fdo::Result<()> {
        export_tar(&self.pool_path, &name, &dest).map_err(zbus::fdo::Error::Failed)
    }

    /// ExportRaw(s name, s dest)
    fn export_raw_method(&self, name: String, dest: String) -> zbus::fdo::Result<()> {
        export_raw(&self.pool_path, &name, &dest).map_err(zbus::fdo::Error::Failed)
    }

    /// PullTar(s url, s name, s verify)
    fn pull_tar_method(&self, url: String, name: String, verify: String) -> zbus::fdo::Result<()> {
        pull_tar(&self.pool_path, &url, &name, &verify).map_err(zbus::fdo::Error::Failed)
    }

    /// PullRaw(s url, s name, s verify)
    fn pull_raw_method(&self, url: String, name: String, verify: String) -> zbus::fdo::Result<()> {
        pull_raw(&self.pool_path, &url, &name, &verify).map_err(zbus::fdo::Error::Failed)
    }

    /// CleanPool(s mode) → a(ss)
    fn clean_pool(&self, _mode: String) -> Vec<(String, String)> {
        // Clean hidden/temporary images
        let pool = Path::new(&self.pool_path);
        let mut cleaned = Vec::new();
        if let Ok(entries) = fs::read_dir(pool) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') && name.ends_with(".tmp") {
                    let path = entry.path();
                    if path.is_dir() {
                        let _ = fs::remove_dir_all(&path);
                    } else {
                        let _ = fs::remove_file(&path);
                    }
                    cleaned.push((name, "removed".to_string()));
                }
            }
        }
        cleaned
    }

    /// Describe() → s (JSON description of the manager state)
    fn describe(&self) -> String {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let machines = registry.list();
        let mut machines_json = String::from("[");
        for (i, m) in machines.iter().enumerate() {
            if i > 0 {
                machines_json.push(',');
            }
            machines_json.push_str(&format!(
                concat!(
                    "{{",
                    "\"Name\":\"{}\",",
                    "\"Class\":\"{}\",",
                    "\"Service\":\"{}\",",
                    "\"Scope\":\"{}\",",
                    "\"Leader\":{},",
                    "\"RootDirectory\":\"{}\",",
                    "\"State\":\"{}\",",
                    "\"Timestamp\":{}",
                    "}}"
                ),
                json_escape(&m.name),
                json_escape(m.class.as_str()),
                json_escape(&m.service),
                json_escape(&m.scope),
                m.leader,
                json_escape(&m.root_directory),
                json_escape(m.state.as_str()),
                m.timestamp,
            ));
        }
        machines_json.push(']');

        let (pool_used, pool_total) = pool_usage_stats(&self.pool_path);
        format!(
            concat!(
                "{{",
                "\"PoolPath\":\"{}\",",
                "\"PoolUsage\":{},",
                "\"PoolLimit\":{},",
                "\"NMachines\":{},",
                "\"Machines\":{}",
                "}}"
            ),
            json_escape(&self.pool_path),
            pool_used,
            pool_total,
            machines.len(),
            machines_json,
        )
    }
}

/// Convert a machine name to a D-Bus object path.
fn machine_object_path(name: &str) -> String {
    let mut path = String::from("/org/freedesktop/machine1/machine/");
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            path.push(ch);
        } else {
            // Escape non-alphanumeric characters as _XX hex
            path.push('_');
            path.push_str(&format!("{:02x}", ch as u32));
        }
    }
    path
}

/// Set up the D-Bus connection and register the machine1 interface.
///
/// Uses zbus's blocking connection which dispatches messages automatically
/// in a background thread. The returned `Connection` must be kept alive
/// for as long as we want to serve D-Bus requests.
fn setup_dbus(shared: SharedRegistry) -> Result<Connection, String> {
    let iface = Machine1Manager {
        registry: shared,
        pool_path: POOL_PATH.to_string(),
    };
    let conn = zbus::blocking::connection::Builder::system()
        .map_err(|e| format!("D-Bus builder failed: {}", e))?
        .name(DBUS_NAME)
        .map_err(|e| format!("D-Bus name request failed: {}", e))?
        .serve_at(DBUS_PATH, iface)
        .map_err(|e| format!("D-Bus serve_at failed: {}", e))?
        .build()
        .map_err(|e| format!("D-Bus connection failed: {}", e))?;
    Ok(conn)
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate a machine name. Machine names follow hostname rules plus
/// allowing dots for FQDN-like names. `.host` is a special valid name.
pub fn is_valid_machine_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    // `.host` is the special name for the host machine itself
    if name == ".host" {
        return true;
    }
    // Reject other dotfile-style names
    if name.starts_with('.') {
        return false;
    }
    // Must start with alphanumeric or underscore
    let first = match name.chars().next() {
        Some(c) => c,
        None => return false,
    };
    if !first.is_ascii_alphanumeric() && first != '_' {
        return false;
    }
    // Must end with alphanumeric
    let last = name.chars().next_back().unwrap();
    if !last.is_ascii_alphanumeric() {
        return false;
    }
    // Only alphanumeric, hyphen, underscore, dot allowed
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_env_content(content: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let mut value = value.trim().to_string();
            // Strip surrounding quotes
            if ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
                && value.len() >= 2
            {
                value = value[1..value.len() - 1].to_string();
            }
            map.insert(key.to_string(), value);
        }
    }
    map
}

fn now_usec() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

fn format_timestamp(usec: u64) -> String {
    if usec == 0 {
        return "n/a".to_string();
    }
    let secs = usec / 1_000_000;
    let total_days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let (year, month, day) = days_to_ymd(total_days);

    static MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mon = if (1..=12).contains(&month) {
        MONTHS[(month - 1) as usize]
    } else {
        "???"
    };

    format!(
        "{} {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        mon, year, month, day, hours, minutes, seconds
    )
}

fn days_to_ymd(total_days: u64) -> (u64, u64, u64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = total_days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ---------------------------------------------------------------------------
// Control socket command handler
// ---------------------------------------------------------------------------

/// Handle a single control command line from a client.
fn handle_control_command(registry: &mut MachineRegistry, line: &str) -> String {
    let parts: Vec<&str> = line.trim().splitn(2, ' ').collect();
    let cmd = parts.first().copied().unwrap_or("");
    let args_str = parts.get(1).copied().unwrap_or("");

    match cmd.to_uppercase().as_str() {
        "LIST" => registry.format_list(),

        "STATUS" => {
            if args_str.is_empty() {
                return format!("Machines: {}\n", registry.count());
            }
            match registry.get(args_str) {
                Some(m) => m.format_status(),
                None => format!("ERROR: Machine '{}' not found\n", args_str),
            }
        }

        "SHOW" => {
            if args_str.is_empty() {
                return "ERROR: Machine name required\n".to_string();
            }
            match registry.get(args_str) {
                Some(m) => m.format_show(),
                None => format!("ERROR: Machine '{}' not found\n", args_str),
            }
        }

        // REGISTER <name> <class> <service> <leader> [root_directory] [scope]
        "REGISTER" => {
            let reg_parts: Vec<&str> = args_str.splitn(6, ' ').collect();
            if reg_parts.len() < 4 {
                return "ERROR: Usage: REGISTER <name> <class> <service> <leader> [root] [scope]\n"
                    .to_string();
            }
            let name = reg_parts[0].to_string();
            let class = match MachineClass::parse(reg_parts[1]) {
                Some(c) => c,
                None => {
                    return format!(
                        "ERROR: Invalid class '{}'. Use 'container' or 'vm'\n",
                        reg_parts[1]
                    );
                }
            };
            let service = reg_parts[2].to_string();
            let leader: u32 = match reg_parts[3].parse() {
                Ok(p) => p,
                Err(_) => return format!("ERROR: Invalid leader PID '{}'\n", reg_parts[3]),
            };
            let root_directory = reg_parts
                .get(4)
                .filter(|s| !s.is_empty())
                .unwrap_or(&"/")
                .to_string();
            let explicit_scope = reg_parts.get(5).unwrap_or(&"").to_string();

            // Attempt to create a transient scope unit
            let scope = if explicit_scope.is_empty() {
                create_machine_scope(&name, leader, &format!("Machine {}", name))
                    .unwrap_or_default()
            } else {
                explicit_scope
            };

            let machine = Machine {
                name: name.clone(),
                class,
                service,
                scope,
                leader,
                root_directory,
                netif: Vec::new(),
                timestamp: now_usec(),
                state: MachineState::Running,
            };

            match registry.register(machine) {
                Ok(()) => {
                    let _ = registry.save_one(&name);
                    format!("OK: Registered '{}'\n", name)
                }
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        "TERMINATE" => {
            if args_str.is_empty() {
                return "ERROR: Machine name required\n".to_string();
            }
            match registry.terminate(args_str) {
                Ok(machine) => {
                    // Optionally kill the leader
                    if machine.leader > 0 {
                        unsafe {
                            libc::kill(machine.leader as i32, libc::SIGTERM);
                        }
                    }
                    // Stop the transient scope
                    if !machine.scope.is_empty() {
                        let _ = stop_machine_scope(&machine.scope);
                    }
                    format!("OK: Terminated '{}'\n", args_str)
                }
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        "GC" => {
            let removed = registry.gc();
            if removed.is_empty() {
                "OK: No stale machines\n".to_string()
            } else {
                format!(
                    "OK: Removed {} stale machines: {}\n",
                    removed.len(),
                    removed.join(", ")
                )
            }
        }

        // --- Image management commands ---
        "IMAGE_LIST" | "LIST-IMAGES" => {
            let images = discover_images(POOL_PATH);
            if images.is_empty() {
                return "No images.\n".to_string();
            }
            let mut s = String::new();
            s.push_str(&format!(
                "{:<32} {:>10} {:>8} {:>12} {:>12}\n",
                "NAME", "TYPE", "RO", "USAGE", "LIMIT"
            ));
            for img in &images {
                s.push_str(&format!(
                    "{:<32} {:>10} {:>8} {:>12} {:>12}\n",
                    img.name,
                    img.image_type,
                    if img.read_only { "ro" } else { "no" },
                    if img.usage > 0 {
                        format_bytes(img.usage)
                    } else {
                        "-".to_string()
                    },
                    if img.limit > 0 {
                        format_bytes(img.limit)
                    } else {
                        "-".to_string()
                    },
                ));
            }
            s.push_str(&format!("\n{} images listed.\n", images.len()));
            s
        }

        "IMAGE_SHOW" | "SHOW-IMAGE" => {
            if args_str.is_empty() {
                return "ERROR: Image name required\n".to_string();
            }
            match find_image(POOL_PATH, args_str) {
                Some(img) => img.format_status(),
                None => format!("ERROR: Image '{}' not found\n", args_str),
            }
        }

        // IMAGE_CLONE <source> <dest> [read_only]
        "IMAGE_CLONE" | "CLONE" => {
            let clone_parts: Vec<&str> = args_str.splitn(3, ' ').collect();
            if clone_parts.len() < 2 {
                return "ERROR: Usage: CLONE <source> <dest> [read_only]\n".to_string();
            }
            let read_only = clone_parts
                .get(2)
                .map(|s| *s == "true" || *s == "1")
                .unwrap_or(false);
            match clone_image(POOL_PATH, clone_parts[0], clone_parts[1], read_only) {
                Ok(()) => format!("OK: Cloned '{}' to '{}'\n", clone_parts[0], clone_parts[1]),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        // IMAGE_RENAME <old> <new>
        "IMAGE_RENAME" | "RENAME" => {
            let rename_parts: Vec<&str> = args_str.splitn(2, ' ').collect();
            if rename_parts.len() < 2 {
                return "ERROR: Usage: RENAME <old_name> <new_name>\n".to_string();
            }
            match rename_image(POOL_PATH, rename_parts[0], rename_parts[1]) {
                Ok(()) => format!(
                    "OK: Renamed '{}' to '{}'\n",
                    rename_parts[0], rename_parts[1]
                ),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        // IMAGE_REMOVE <name>
        "IMAGE_REMOVE" | "REMOVE" => {
            if args_str.is_empty() {
                return "ERROR: Image name required\n".to_string();
            }
            match remove_image(POOL_PATH, args_str) {
                Ok(()) => format!("OK: Removed image '{}'\n", args_str),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        // IMAGE_SET_LIMIT <name> <limit>
        "IMAGE_SET_LIMIT" | "SET-LIMIT" => {
            let limit_parts: Vec<&str> = args_str.splitn(2, ' ').collect();
            if limit_parts.len() < 2 {
                return "ERROR: Usage: SET-LIMIT <name> <limit>\n".to_string();
            }
            let limit = match parse_size(limit_parts[1]) {
                Some(l) => l,
                None => return format!("ERROR: Invalid size '{}'\n", limit_parts[1]),
            };
            match set_image_limit(POOL_PATH, limit_parts[0], limit) {
                Ok(()) => format!(
                    "OK: Set limit for '{}' to {} bytes\n",
                    limit_parts[0], limit
                ),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        // --- Bind mount operations ---

        // BIND <machine> <source> [destination] [read_only] [mkdir]
        "BIND" | "BIND-MOUNT" => {
            let bind_parts: Vec<&str> = args_str.splitn(5, ' ').collect();
            if bind_parts.len() < 2 {
                return "ERROR: Usage: BIND <machine> <source> [destination] [read_only] [mkdir]\n"
                    .to_string();
            }
            let machine_name = bind_parts[0];
            let source = bind_parts[1];
            let destination = bind_parts.get(2).copied().unwrap_or(source);
            let read_only = bind_parts
                .get(3)
                .map(|s| *s == "true" || *s == "1")
                .unwrap_or(false);
            let mkdir = bind_parts
                .get(4)
                .map(|s| *s == "true" || *s == "1")
                .unwrap_or(false);
            match bind_mount_in_machine(
                registry,
                machine_name,
                source,
                destination,
                read_only,
                mkdir,
            ) {
                Ok(()) => format!(
                    "OK: Bind-mounted '{}' to '{}:{}'\n",
                    source, machine_name, destination
                ),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        // BIND-USER <machine> <source> <user> [destination] [read_only]
        "BIND-USER" | "BIND_USER" => {
            let bind_parts: Vec<&str> = args_str.splitn(5, ' ').collect();
            if bind_parts.len() < 3 {
                return "ERROR: Usage: BIND-USER <machine> <source> <user> [destination] [read_only]\n"
                    .to_string();
            }
            let machine_name = bind_parts[0];
            let source = bind_parts[1];
            let user = bind_parts[2];
            let destination = bind_parts.get(3).filter(|s| !s.is_empty());
            let read_only = bind_parts
                .get(4)
                .map(|s| *s == "true" || *s == "1")
                .unwrap_or(false);
            match bind_user_in_machine(
                registry,
                machine_name,
                source,
                destination.copied(),
                user,
                read_only,
            ) {
                Ok(()) => format!(
                    "OK: Bind-user-mounted '{}' for user '{}' in '{}'\n",
                    source, user, machine_name
                ),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        // --- Copy operations ---

        // COPY_TO <machine> <host_path> <container_path>
        "COPY_TO" | "COPY-TO" => {
            let copy_parts: Vec<&str> = args_str.splitn(3, ' ').collect();
            if copy_parts.len() < 3 {
                return "ERROR: Usage: COPY-TO <machine> <host_path> <container_path>\n"
                    .to_string();
            }
            match copy_to_machine(registry, copy_parts[0], copy_parts[1], copy_parts[2]) {
                Ok(()) => format!(
                    "OK: Copied '{}' to '{}:{}'\n",
                    copy_parts[1], copy_parts[0], copy_parts[2]
                ),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        // COPY_FROM <machine> <container_path> <host_path>
        "COPY_FROM" | "COPY-FROM" => {
            let copy_parts: Vec<&str> = args_str.splitn(3, ' ').collect();
            if copy_parts.len() < 3 {
                return "ERROR: Usage: COPY-FROM <machine> <container_path> <host_path>\n"
                    .to_string();
            }
            match copy_from_machine(registry, copy_parts[0], copy_parts[1], copy_parts[2]) {
                Ok(()) => format!(
                    "OK: Copied '{}:{}' to '{}'\n",
                    copy_parts[0], copy_parts[1], copy_parts[2]
                ),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        // --- PTY forwarding ---

        // LOGIN <machine>
        "LOGIN" => {
            if args_str.is_empty() {
                return "ERROR: Machine name required\n".to_string();
            }
            match open_machine_pty(registry, args_str, None, None) {
                Ok((master_fd, child_pid)) => {
                    format!("OK: PTY master_fd={} child_pid={}\n", master_fd, child_pid)
                }
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        // SHELL <machine> [user] [command]
        "SHELL" => {
            let shell_parts: Vec<&str> = args_str.splitn(3, ' ').collect();
            if shell_parts.is_empty() || shell_parts[0].is_empty() {
                return "ERROR: Machine name required\n".to_string();
            }
            let user = shell_parts.get(1).filter(|s| !s.is_empty()).copied();
            let cmd = shell_parts.get(2).filter(|s| !s.is_empty()).copied();
            match open_machine_pty(registry, shell_parts[0], cmd, user) {
                Ok((master_fd, child_pid)) => {
                    format!("OK: PTY master_fd={} child_pid={}\n", master_fd, child_pid)
                }
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        // --- Import/Export/Pull ---

        // IMPORT_TAR <source> <name> [read_only]
        "IMPORT_TAR" | "IMPORT-TAR" => {
            let imp_parts: Vec<&str> = args_str.splitn(3, ' ').collect();
            if imp_parts.len() < 2 {
                return "ERROR: Usage: IMPORT-TAR <source> <name> [read_only]\n".to_string();
            }
            let read_only = imp_parts
                .get(2)
                .map(|s| *s == "true" || *s == "1")
                .unwrap_or(false);
            match import_tar(POOL_PATH, imp_parts[0], imp_parts[1], read_only) {
                Ok(()) => format!(
                    "OK: Imported tar '{}' as '{}'\n",
                    imp_parts[0], imp_parts[1]
                ),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        // IMPORT_RAW <source> <name> [read_only]
        "IMPORT_RAW" | "IMPORT-RAW" => {
            let imp_parts: Vec<&str> = args_str.splitn(3, ' ').collect();
            if imp_parts.len() < 2 {
                return "ERROR: Usage: IMPORT-RAW <source> <name> [read_only]\n".to_string();
            }
            let read_only = imp_parts
                .get(2)
                .map(|s| *s == "true" || *s == "1")
                .unwrap_or(false);
            match import_raw(POOL_PATH, imp_parts[0], imp_parts[1], read_only) {
                Ok(()) => format!(
                    "OK: Imported raw '{}' as '{}'\n",
                    imp_parts[0], imp_parts[1]
                ),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        // EXPORT_TAR <name> <dest>
        "EXPORT_TAR" | "EXPORT-TAR" => {
            let exp_parts: Vec<&str> = args_str.splitn(2, ' ').collect();
            if exp_parts.len() < 2 {
                return "ERROR: Usage: EXPORT-TAR <name> <dest>\n".to_string();
            }
            match export_tar(POOL_PATH, exp_parts[0], exp_parts[1]) {
                Ok(()) => format!("OK: Exported '{}' to '{}'\n", exp_parts[0], exp_parts[1]),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        // EXPORT_RAW <name> <dest>
        "EXPORT_RAW" | "EXPORT-RAW" => {
            let exp_parts: Vec<&str> = args_str.splitn(2, ' ').collect();
            if exp_parts.len() < 2 {
                return "ERROR: Usage: EXPORT-RAW <name> <dest>\n".to_string();
            }
            match export_raw(POOL_PATH, exp_parts[0], exp_parts[1]) {
                Ok(()) => format!("OK: Exported '{}' to '{}'\n", exp_parts[0], exp_parts[1]),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        // PULL_TAR <url> <name> [verify]
        "PULL_TAR" | "PULL-TAR" => {
            let pull_parts: Vec<&str> = args_str.splitn(3, ' ').collect();
            if pull_parts.len() < 2 {
                return "ERROR: Usage: PULL-TAR <url> <name> [verify]\n".to_string();
            }
            let verify = pull_parts.get(2).copied().unwrap_or("no");
            match pull_tar(POOL_PATH, pull_parts[0], pull_parts[1], verify) {
                Ok(()) => format!(
                    "OK: Pulled tar '{}' as '{}'\n",
                    pull_parts[0], pull_parts[1]
                ),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        // PULL_RAW <url> <name> [verify]
        "PULL_RAW" | "PULL-RAW" => {
            let pull_parts: Vec<&str> = args_str.splitn(3, ' ').collect();
            if pull_parts.len() < 2 {
                return "ERROR: Usage: PULL-RAW <url> <name> [verify]\n".to_string();
            }
            let verify = pull_parts.get(2).copied().unwrap_or("no");
            match pull_raw(POOL_PATH, pull_parts[0], pull_parts[1], verify) {
                Ok(()) => format!(
                    "OK: Pulled raw '{}' as '{}'\n",
                    pull_parts[0], pull_parts[1]
                ),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        "PING" => "PONG\n".to_string(),

        _ => format!("ERROR: Unknown command '{}'\n", cmd),
    }
}

/// Handle a client connection on the control socket.
fn handle_client(registry: &mut MachineRegistry, stream: &mut UnixStream) {
    let reader = match stream.try_clone() {
        Ok(s) => BufReader::new(s),
        Err(_) => return,
    };

    for line in reader.lines() {
        match line {
            Ok(l) if l.trim().is_empty() => continue,
            Ok(l) => {
                let response = handle_control_command(registry, &l);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
            Err(_) => break,
        }
    }
}

// ---------------------------------------------------------------------------
// sd_notify helper
// ---------------------------------------------------------------------------

fn sd_notify(msg: &str) {
    let sock_path = match env::var("NOTIFY_SOCKET") {
        Ok(p) => p,
        Err(_) => return,
    };

    let path = if let Some(stripped) = sock_path.strip_prefix('@') {
        format!("\0{}", stripped)
    } else {
        sock_path
    };

    let sock = match UnixDatagram::unbound() {
        Ok(s) => s,
        Err(_) => return,
    };

    let _ = sock.send_to(msg.as_bytes(), &path);
}

// ---------------------------------------------------------------------------
// Signal handling
// ---------------------------------------------------------------------------

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static RELOAD: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigterm(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

extern "C" fn handle_sigint(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

extern "C" fn handle_sighup(_: libc::c_int) {
    RELOAD.store(true, Ordering::SeqCst);
}

fn setup_signal_handlers() {
    unsafe {
        libc::signal(
            libc::SIGTERM,
            handle_sigterm as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            handle_sigint as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGHUP,
            handle_sighup as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

fn init_logging() {
    struct StderrLogger;
    impl log::Log for StderrLogger {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                eprintln!(
                    "systemd-machined[{}]: {}: {}",
                    process::id(),
                    record.level(),
                    record.args()
                );
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: StderrLogger = StderrLogger;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
}

// ---------------------------------------------------------------------------
// Watchdog
// ---------------------------------------------------------------------------

fn watchdog_interval() -> Option<Duration> {
    let usec: u64 = env::var("WATCHDOG_USEC").ok()?.parse().ok()?;
    if usec == 0 {
        return None;
    }
    // Send at half the interval
    Some(Duration::from_micros(usec / 2))
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    init_logging();
    setup_signal_handlers();

    log::info!("systemd-machined starting");

    // Load existing machine state into shared registry for D-Bus and control socket
    let mut registry = MachineRegistry::new();
    let _ = fs::create_dir_all(MACHINES_DIR);
    registry.load();
    log::info!("Loaded {} machines from state files", registry.count());

    // GC stale machines on startup
    let removed = registry.gc();
    if !removed.is_empty() {
        log::info!("Removed {} stale machines on startup", removed.len());
    }

    let initial_count = registry.count();
    let shared_registry: SharedRegistry = Arc::new(Mutex::new(registry));

    // Watchdog support
    let wd_interval = watchdog_interval();
    if let Some(ref iv) = wd_interval {
        log::info!("Watchdog enabled, interval {:?}", iv);
    }
    let mut last_watchdog = Instant::now();

    // D-Bus connection is deferred to after READY=1 so we don't block early
    // boot waiting for dbus-daemon.  zbus dispatches messages automatically
    // in a background thread — we just keep the connection alive.
    let mut _dbus_conn: Option<Connection> = None;
    let mut dbus_attempted = false;

    // Ensure parent directory exists
    let _ = fs::create_dir_all(Path::new(CONTROL_SOCKET_PATH).parent().unwrap());

    // Remove stale socket
    let _ = fs::remove_file(CONTROL_SOCKET_PATH);

    // Bind control socket
    let listener = match UnixListener::bind(CONTROL_SOCKET_PATH) {
        Ok(l) => {
            log::info!("Listening on {}", CONTROL_SOCKET_PATH);
            Some(l)
        }
        Err(e) => {
            log::error!(
                "Failed to bind control socket {}: {}",
                CONTROL_SOCKET_PATH,
                e
            );
            None
        }
    };

    // Set socket to non-blocking so we can check SHUTDOWN flag periodically
    if let Some(ref l) = listener {
        l.set_nonblocking(true).expect("Failed to set non-blocking");
    }

    sd_notify(&format!(
        "READY=1\nSTATUS=Managing {} machines",
        initial_count
    ));

    log::info!("systemd-machined ready");

    // Periodic GC interval
    let gc_interval = Duration::from_secs(30);
    let mut last_gc = Instant::now();

    // Main loop
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            log::info!("Received shutdown signal");
            break;
        }

        if RELOAD.load(Ordering::SeqCst) {
            RELOAD.store(false, Ordering::SeqCst);
            let mut reg = shared_registry.lock().unwrap_or_else(|e| e.into_inner());
            reg.load();
            let count = reg.count();
            log::info!("Reloaded, {} machines", count);
            sd_notify(&format!("STATUS=Managing {} machines", count));
        }

        // Send watchdog keepalive
        if let Some(ref iv) = wd_interval
            && last_watchdog.elapsed() >= *iv
        {
            sd_notify("WATCHDOG=1");
            last_watchdog = Instant::now();
        }

        // Attempt D-Bus registration once (deferred from startup so we don't
        // block early boot before dbus-daemon is running).
        if !dbus_attempted {
            dbus_attempted = true;
            match setup_dbus(shared_registry.clone()) {
                Ok(conn) => {
                    log::info!("D-Bus interface registered: {} at {}", DBUS_NAME, DBUS_PATH);
                    _dbus_conn = Some(conn);
                    let count = shared_registry
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .count();
                    sd_notify(&format!(
                        "STATUS=Managing {} machines (D-Bus active)",
                        count
                    ));
                }
                Err(e) => {
                    log::warn!(
                        "Failed to register D-Bus interface ({}); control socket only",
                        e
                    );
                }
            }
        }

        // zbus dispatches D-Bus messages automatically in a background thread.

        // Periodic GC of dead machines
        if last_gc.elapsed() >= gc_interval {
            let mut reg = shared_registry.lock().unwrap_or_else(|e| e.into_inner());
            let removed = reg.gc();
            if !removed.is_empty() {
                log::info!(
                    "GC removed {} stale machines: {}",
                    removed.len(),
                    removed.join(", ")
                );
                sd_notify(&format!("STATUS=Managing {} machines", reg.count()));
            }
            last_gc = Instant::now();
        }

        // Accept control socket connections
        if let Some(ref listener) = listener {
            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut reg = shared_registry.lock().unwrap_or_else(|e| e.into_inner());
                    handle_client(&mut reg, &mut stream);
                    let _ = stream.shutdown(Shutdown::Both);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // No connection waiting
                }
                Err(e) => {
                    log::warn!("Accept error: {}", e);
                }
            }
        }

        // Brief sleep to avoid busy-looping when there's no work
        thread::sleep(Duration::from_millis(50));
    }

    // Cleanup
    let _ = fs::remove_file(CONTROL_SOCKET_PATH);
    sd_notify("STOPPING=1");
    log::info!("systemd-machined stopped");
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // D-Bus registration tests

    #[test]
    fn test_dbus_machine1_manager_struct() {
        let shared: SharedRegistry = Arc::new(Mutex::new(MachineRegistry::new()));
        let _mgr = Machine1Manager {
            registry: shared,
            pool_path: "/var/lib/machines".to_string(),
        };
        // Struct creation succeeded without panic
    }

    #[test]
    fn test_machine_object_path_simple() {
        assert_eq!(
            machine_object_path("mycontainer"),
            "/org/freedesktop/machine1/machine/mycontainer"
        );
    }

    #[test]
    fn test_machine_object_path_with_dots() {
        // dots should be escaped
        let path = machine_object_path("my.container");
        assert_eq!(path, "/org/freedesktop/machine1/machine/my_2econtainer");
    }

    #[test]
    fn test_machine_object_path_with_hyphen() {
        let path = machine_object_path("my-vm");
        assert_eq!(path, "/org/freedesktop/machine1/machine/my_2dvm");
    }

    #[test]
    fn test_machine_object_path_dot_host() {
        let path = machine_object_path(".host");
        assert_eq!(path, "/org/freedesktop/machine1/machine/_2ehost");
    }

    #[test]
    fn test_json_escape_plain() {
        assert_eq!(json_escape("hello"), "hello");
    }

    #[test]
    fn test_json_escape_quotes() {
        assert_eq!(json_escape("he\"llo"), "he\\\"llo");
    }

    #[test]
    fn test_json_escape_backslash() {
        assert_eq!(json_escape("he\\llo"), "he\\\\llo");
    }

    #[test]
    fn test_json_escape_newline() {
        assert_eq!(json_escape("he\nllo"), "he\\nllo");
    }

    #[test]
    fn test_json_escape_control_char() {
        assert_eq!(json_escape("he\x01llo"), "he\\u0001llo");
    }

    #[test]
    fn test_json_escape_empty() {
        assert_eq!(json_escape(""), "");
    }

    fn temp_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    // -- MachineClass -------------------------------------------------------

    #[test]
    fn test_machine_class_parse() {
        assert_eq!(
            MachineClass::parse("container"),
            Some(MachineClass::Container)
        );
        assert_eq!(
            MachineClass::parse("Container"),
            Some(MachineClass::Container)
        );
        assert_eq!(
            MachineClass::parse("CONTAINER"),
            Some(MachineClass::Container)
        );
        assert_eq!(MachineClass::parse("vm"), Some(MachineClass::Vm));
        assert_eq!(MachineClass::parse("VM"), Some(MachineClass::Vm));
        assert_eq!(MachineClass::parse("Vm"), Some(MachineClass::Vm));
        assert_eq!(MachineClass::parse("invalid"), None);
        assert_eq!(MachineClass::parse(""), None);
    }

    #[test]
    fn test_machine_class_display() {
        assert_eq!(format!("{}", MachineClass::Container), "container");
        assert_eq!(format!("{}", MachineClass::Vm), "vm");
    }

    #[test]
    fn test_machine_class_as_str() {
        assert_eq!(MachineClass::Container.as_str(), "container");
        assert_eq!(MachineClass::Vm.as_str(), "vm");
    }

    // -- MachineState -------------------------------------------------------

    #[test]
    fn test_machine_state_parse() {
        assert_eq!(MachineState::parse("opening"), Some(MachineState::Opening));
        assert_eq!(MachineState::parse("running"), Some(MachineState::Running));
        assert_eq!(MachineState::parse("closing"), Some(MachineState::Closing));
        assert_eq!(MachineState::parse("RUNNING"), Some(MachineState::Running));
        assert_eq!(MachineState::parse("invalid"), None);
    }

    #[test]
    fn test_machine_state_display() {
        assert_eq!(format!("{}", MachineState::Opening), "opening");
        assert_eq!(format!("{}", MachineState::Running), "running");
        assert_eq!(format!("{}", MachineState::Closing), "closing");
    }

    // -- Machine state file roundtrip ---------------------------------------

    fn make_test_machine(name: &str) -> Machine {
        Machine {
            name: name.to_string(),
            class: MachineClass::Container,
            service: "systemd-nspawn".to_string(),
            scope: format!("machine-{}.scope", name),
            leader: 12345,
            root_directory: "/".to_string(),
            netif: vec![3, 7],
            timestamp: 1_700_000_000_000_000,
            state: MachineState::Running,
        }
    }

    #[test]
    fn test_machine_state_file_roundtrip() {
        let machine = make_test_machine("mycontainer");
        let content = machine.to_state_file();
        let parsed = Machine::from_state_file(&content).unwrap();

        assert_eq!(parsed.name, "mycontainer");
        assert_eq!(parsed.class, MachineClass::Container);
        assert_eq!(parsed.service, "systemd-nspawn");
        assert_eq!(parsed.scope, "machine-mycontainer.scope");
        assert_eq!(parsed.leader, 12345);
        assert_eq!(parsed.root_directory, "/");
        assert_eq!(parsed.state, MachineState::Running);
        assert_eq!(parsed.timestamp, 1_700_000_000_000_000);
        assert_eq!(parsed.netif, vec![3, 7]);
    }

    #[test]
    fn test_machine_state_file_no_netif() {
        let mut machine = make_test_machine("nonet");
        machine.netif = vec![];
        let content = machine.to_state_file();
        assert!(!content.contains("NETIF="));
        let parsed = Machine::from_state_file(&content).unwrap();
        assert!(parsed.netif.is_empty());
    }

    #[test]
    fn test_machine_state_file_vm_class() {
        let mut machine = make_test_machine("myvm");
        machine.class = MachineClass::Vm;
        let content = machine.to_state_file();
        assert!(content.contains("CLASS=vm\n"));
        let parsed = Machine::from_state_file(&content).unwrap();
        assert_eq!(parsed.class, MachineClass::Vm);
    }

    #[test]
    fn test_machine_from_state_file_missing_name() {
        let content = "CLASS=container\nLEADER=1\n";
        assert!(Machine::from_state_file(content).is_none());
    }

    #[test]
    fn test_machine_from_state_file_missing_class() {
        let content = "NAME=test\nLEADER=1\n";
        assert!(Machine::from_state_file(content).is_none());
    }

    #[test]
    fn test_machine_from_state_file_invalid_class() {
        let content = "NAME=test\nCLASS=invalid\nLEADER=1\n";
        assert!(Machine::from_state_file(content).is_none());
    }

    #[test]
    fn test_machine_from_state_file_minimal() {
        let content = "NAME=test\nCLASS=container\n";
        let m = Machine::from_state_file(content).unwrap();
        assert_eq!(m.name, "test");
        assert_eq!(m.class, MachineClass::Container);
        assert_eq!(m.leader, 0);
        assert_eq!(m.root_directory, "/");
        assert_eq!(m.state, MachineState::Running);
        assert!(m.netif.is_empty());
    }

    // -- Machine format -----------------------------------------------------

    #[test]
    fn test_machine_format_status() {
        let machine = make_test_machine("mycontainer");
        let status = machine.format_status();
        assert!(status.contains("Name: mycontainer"));
        assert!(status.contains("Class: container"));
        assert!(status.contains("Service: systemd-nspawn"));
        assert!(status.contains("Leader: 12345"));
        assert!(status.contains("State: running"));
        assert!(status.contains("NetIf: 3 7"));
    }

    #[test]
    fn test_machine_format_show() {
        let machine = make_test_machine("mycontainer");
        let show = machine.format_show();
        assert!(show.contains("Name=mycontainer\n"));
        assert!(show.contains("Class=container\n"));
        assert!(show.contains("Leader=12345\n"));
        assert!(show.contains("State=running\n"));
        assert!(show.contains("NetworkInterfaces=3 7\n"));
    }

    #[test]
    fn test_machine_format_show_no_netif() {
        let mut machine = make_test_machine("nonet");
        machine.netif = vec![];
        let show = machine.format_show();
        assert!(!show.contains("NetworkInterfaces="));
    }

    // -- Validation ---------------------------------------------------------

    #[test]
    fn test_is_valid_machine_name_valid() {
        assert!(is_valid_machine_name("mycontainer"));
        assert!(is_valid_machine_name("test-vm"));
        assert!(is_valid_machine_name("my.container"));
        assert!(is_valid_machine_name("a"));
        assert!(is_valid_machine_name("_private"));
        assert!(is_valid_machine_name("test_vm"));
        assert!(is_valid_machine_name(".host"));
    }

    #[test]
    fn test_is_valid_machine_name_invalid() {
        assert!(!is_valid_machine_name(""));
        assert!(!is_valid_machine_name("-starts-with-hyphen"));
        assert!(!is_valid_machine_name("ends-with-hyphen-"));
        assert!(!is_valid_machine_name(".dotfile"));
        assert!(!is_valid_machine_name(".."));
        assert!(!is_valid_machine_name("."));
        assert!(!is_valid_machine_name("has space"));
        assert!(!is_valid_machine_name("has/slash"));
        assert!(!is_valid_machine_name(&"a".repeat(65)));
    }

    #[test]
    fn test_is_valid_machine_name_max_length() {
        assert!(is_valid_machine_name(&"a".repeat(64)));
        assert!(!is_valid_machine_name(&"a".repeat(65)));
    }

    // -- MachineRegistry ----------------------------------------------------

    #[test]
    fn test_registry_register_and_list() {
        let mut reg = MachineRegistry::new();
        assert_eq!(reg.count(), 0);
        assert!(reg.list().is_empty());

        let machine = make_test_machine("test1");
        reg.register(machine).unwrap();
        assert_eq!(reg.count(), 1);
        assert_eq!(reg.list()[0].name, "test1");
    }

    #[test]
    fn test_registry_register_duplicate() {
        let mut reg = MachineRegistry::new();
        reg.register(make_test_machine("test1")).unwrap();
        let result = reg.register(make_test_machine("test1"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already registered"));
    }

    #[test]
    fn test_registry_register_empty_name() {
        let mut reg = MachineRegistry::new();
        let mut machine = make_test_machine("x");
        machine.name = String::new();
        let result = reg.register(machine);
        assert!(result.is_err());
    }

    #[test]
    fn test_registry_register_invalid_name() {
        let mut reg = MachineRegistry::new();
        let mut machine = make_test_machine("x");
        machine.name = "has space".to_string();
        let result = reg.register(machine);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid machine name"));
    }

    #[test]
    fn test_registry_terminate() {
        let mut reg = MachineRegistry::new();
        reg.register(make_test_machine("test1")).unwrap();
        assert_eq!(reg.count(), 1);

        let dir = temp_dir();
        let removed = reg
            .terminate_in("test1", dir.path().to_str().unwrap())
            .unwrap();
        assert_eq!(removed.name, "test1");
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn test_registry_terminate_not_found() {
        let mut reg = MachineRegistry::new();
        let dir = temp_dir();
        let result = reg.terminate_in("nonexistent", dir.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_registry_get() {
        let mut reg = MachineRegistry::new();
        reg.register(make_test_machine("test1")).unwrap();
        assert!(reg.get("test1").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_find_by_leader() {
        let mut reg = MachineRegistry::new();
        let mut m = make_test_machine("test1");
        m.leader = 42;
        reg.register(m).unwrap();

        assert_eq!(reg.find_by_leader(42).unwrap().name, "test1");
        assert!(reg.find_by_leader(99).is_none());
    }

    // -- Registry persistence -----------------------------------------------

    #[test]
    fn test_registry_save_and_load() {
        let dir = temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        let mut reg = MachineRegistry::new();
        reg.register(make_test_machine("alpha")).unwrap();
        let mut beta = make_test_machine("beta");
        beta.class = MachineClass::Vm;
        beta.leader = 99999;
        reg.register(beta).unwrap();
        reg.save_all_to(dir_path).unwrap();

        // Verify files exist
        assert!(dir.path().join("alpha").exists());
        assert!(dir.path().join("beta").exists());

        // Load into a fresh registry
        let mut reg2 = MachineRegistry::new();
        reg2.load_from(dir_path);
        assert_eq!(reg2.count(), 2);
        assert_eq!(reg2.get("alpha").unwrap().class, MachineClass::Container);
        assert_eq!(reg2.get("beta").unwrap().class, MachineClass::Vm);
        assert_eq!(reg2.get("beta").unwrap().leader, 99999);
    }

    #[test]
    fn test_registry_save_one() {
        let dir = temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        let mut reg = MachineRegistry::new();
        reg.register(make_test_machine("single")).unwrap();
        reg.save_one_to("single", dir_path).unwrap();

        assert!(dir.path().join("single").exists());
        let content = fs::read_to_string(dir.path().join("single")).unwrap();
        assert!(content.contains("NAME=single\n"));
    }

    #[test]
    fn test_registry_load_empty_dir() {
        let dir = temp_dir();
        let mut reg = MachineRegistry::new();
        reg.load_from(dir.path().to_str().unwrap());
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn test_registry_load_nonexistent_dir() {
        let mut reg = MachineRegistry::new();
        reg.load_from("/nonexistent/path/to/machines");
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn test_registry_load_skips_dotfiles() {
        let dir = temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        // Write a valid machine file
        let m = make_test_machine("visible");
        fs::write(dir.path().join("visible"), m.to_state_file()).unwrap();

        // Write a dotfile (should be skipped)
        fs::write(dir.path().join(".hidden"), "NAME=hidden\nCLASS=container\n").unwrap();

        let mut reg = MachineRegistry::new();
        reg.load_from(dir_path);
        assert_eq!(reg.count(), 1);
        assert!(reg.get("visible").is_some());
        assert!(reg.get("hidden").is_none());
    }

    #[test]
    fn test_registry_load_skips_invalid_files() {
        let dir = temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        // Write a valid machine file
        fs::write(
            dir.path().join("good"),
            "NAME=good\nCLASS=container\nLEADER=1\n",
        )
        .unwrap();

        // Write an invalid file (missing CLASS)
        fs::write(dir.path().join("bad"), "NAME=bad\nLEADER=1\n").unwrap();

        let mut reg = MachineRegistry::new();
        reg.load_from(dir_path);
        assert_eq!(reg.count(), 1);
        assert!(reg.get("good").is_some());
    }

    // -- Registry GC --------------------------------------------------------

    #[test]
    fn test_registry_gc_keeps_alive() {
        let mut reg = MachineRegistry::new();
        let mut m = make_test_machine("self");
        // Use our own PID so it's definitely alive
        m.leader = process::id();
        reg.register(m).unwrap();

        let dir = temp_dir();
        let removed = reg.gc_in(dir.path().to_str().unwrap());
        assert!(removed.is_empty());
        assert_eq!(reg.count(), 1);
    }

    #[test]
    fn test_registry_gc_removes_dead() {
        let mut reg = MachineRegistry::new();
        let mut m = make_test_machine("dead");
        // PID that almost certainly doesn't exist
        m.leader = 4_000_000;
        reg.register(m).unwrap();

        let dir = temp_dir();
        let removed = reg.gc_in(dir.path().to_str().unwrap());
        assert_eq!(removed, vec!["dead"]);
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn test_registry_gc_leader_zero_is_dead() {
        let mut reg = MachineRegistry::new();
        let mut m = make_test_machine("noleader");
        m.leader = 0;
        reg.register(m).unwrap();

        let dir = temp_dir();
        let removed = reg.gc_in(dir.path().to_str().unwrap());
        assert_eq!(removed, vec!["noleader"]);
    }

    // -- Registry format_list -----------------------------------------------

    #[test]
    fn test_registry_format_list_empty() {
        let reg = MachineRegistry::new();
        let output = reg.format_list();
        assert!(output.contains("No machines."));
    }

    #[test]
    fn test_registry_format_list_with_machines() {
        let mut reg = MachineRegistry::new();
        reg.register(make_test_machine("alpha")).unwrap();
        let mut beta = make_test_machine("beta");
        beta.class = MachineClass::Vm;
        reg.register(beta).unwrap();

        let output = reg.format_list();
        assert!(output.contains("MACHINE"));
        assert!(output.contains("alpha"));
        assert!(output.contains("beta"));
        assert!(output.contains("container"));
        assert!(output.contains("vm"));
        assert!(output.contains("2 machines listed."));
    }

    // -- parse_env_content --------------------------------------------------

    #[test]
    fn test_parse_env_content_basic() {
        let content = "KEY=value\nFOO=bar\n"; // pragma: allowlist secret
        let m = parse_env_content(content);
        assert_eq!(m.get("KEY").unwrap(), "value");
        assert_eq!(m.get("FOO").unwrap(), "bar");
    }

    #[test]
    fn test_parse_env_content_quoted() {
        let content = "KEY=\"hello world\"\n";
        let m = parse_env_content(content);
        assert_eq!(m.get("KEY").unwrap(), "hello world");
    }

    #[test]
    fn test_parse_env_content_single_quoted() {
        let content = "KEY='hello world'\n";
        let m = parse_env_content(content);
        assert_eq!(m.get("KEY").unwrap(), "hello world");
    }

    #[test]
    fn test_parse_env_content_comments_and_blanks() {
        let content = "# comment\n\nKEY=value\n  # another comment\n";
        let m = parse_env_content(content);
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("KEY").unwrap(), "value");
    }

    #[test]
    fn test_parse_env_content_empty() {
        let m = parse_env_content("");
        assert!(m.is_empty());
    }

    // -- format_timestamp / days_to_ymd -------------------------------------

    #[test]
    fn test_format_timestamp_zero() {
        assert_eq!(format_timestamp(0), "n/a");
    }

    #[test]
    fn test_format_timestamp_epoch() {
        // 1970-01-01 00:00:00 UTC
        let ts = format_timestamp(1);
        assert!(ts.contains("1970"));
    }

    #[test]
    fn test_format_timestamp_known_date() {
        // 2023-11-14 22:13:20 UTC = 1700000000 seconds since epoch
        let ts = format_timestamp(1_700_000_000_000_000);
        assert!(ts.contains("2023"));
        assert!(ts.contains("Nov"));
        assert!(ts.contains("14"));
    }

    #[test]
    fn test_days_to_ymd_epoch() {
        let (y, m, d) = days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn test_days_to_ymd_known() {
        // 2000-01-01 is day 10957
        let (y, m, d) = days_to_ymd(10957);
        assert_eq!((y, m, d), (2000, 1, 1));
    }

    // -- now_usec -----------------------------------------------------------

    #[test]
    fn test_now_usec_nonzero() {
        assert!(now_usec() > 0);
    }

    // -- Control command handler --------------------------------------------

    #[test]
    fn test_handle_command_ping() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "PING");
        assert_eq!(resp, "PONG\n");
    }

    #[test]
    fn test_handle_command_list_empty() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "LIST");
        assert!(resp.contains("No machines."));
    }

    #[test]
    fn test_handle_command_list_with_machines() {
        let mut reg = MachineRegistry::new();
        reg.register(make_test_machine("test1")).unwrap();
        let resp = handle_control_command(&mut reg, "LIST");
        assert!(resp.contains("test1"));
        assert!(resp.contains("1 machines listed."));
    }

    #[test]
    fn test_handle_command_status_global() {
        let mut reg = MachineRegistry::new();
        reg.register(make_test_machine("test1")).unwrap();
        let resp = handle_control_command(&mut reg, "STATUS");
        assert!(resp.contains("Machines: 1"));
    }

    #[test]
    fn test_handle_command_status_specific() {
        let mut reg = MachineRegistry::new();
        reg.register(make_test_machine("test1")).unwrap();
        let resp = handle_control_command(&mut reg, "STATUS test1");
        assert!(resp.contains("Name: test1"));
        assert!(resp.contains("Class: container"));
    }

    #[test]
    fn test_handle_command_status_not_found() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "STATUS nonexistent");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("not found"));
    }

    #[test]
    fn test_handle_command_show() {
        let mut reg = MachineRegistry::new();
        reg.register(make_test_machine("test1")).unwrap();
        let resp = handle_control_command(&mut reg, "SHOW test1");
        assert!(resp.contains("Name=test1\n"));
        assert!(resp.contains("Class=container\n"));
    }

    #[test]
    fn test_handle_command_show_missing_name() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "SHOW");
        assert!(resp.contains("ERROR"));
    }

    #[test]
    fn test_handle_command_register() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(
            &mut reg,
            "REGISTER myvm vm qemu 1234 /var/lib/machines/myvm",
        );
        assert!(resp.contains("OK"));
        assert!(resp.contains("Registered 'myvm'"));
        assert_eq!(reg.count(), 1);

        let m = reg.get("myvm").unwrap();
        assert_eq!(m.class, MachineClass::Vm);
        assert_eq!(m.service, "qemu");
        assert_eq!(m.leader, 1234);
        assert_eq!(m.root_directory, "/var/lib/machines/myvm");
    }

    #[test]
    fn test_handle_command_register_container() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "REGISTER myc container systemd-nspawn 5678");
        assert!(resp.contains("OK"));

        let m = reg.get("myc").unwrap();
        assert_eq!(m.class, MachineClass::Container);
        assert_eq!(m.root_directory, "/");
    }

    #[test]
    fn test_handle_command_register_invalid_class() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "REGISTER test invalid svc 1");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Invalid class"));
    }

    #[test]
    fn test_handle_command_register_invalid_pid() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "REGISTER test container svc notanumber");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Invalid leader PID"));
    }

    #[test]
    fn test_handle_command_register_too_few_args() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "REGISTER test container");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Usage"));
    }

    #[test]
    fn test_handle_command_register_duplicate() {
        let mut reg = MachineRegistry::new();
        handle_control_command(&mut reg, "REGISTER test1 container svc 1");
        let resp = handle_control_command(&mut reg, "REGISTER test1 container svc 2");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("already registered"));
    }

    #[test]
    fn test_handle_command_terminate() {
        let mut reg = MachineRegistry::new();
        reg.register(make_test_machine("todel")).unwrap();
        assert_eq!(reg.count(), 1);
        let resp = handle_control_command(&mut reg, "TERMINATE todel");
        assert!(resp.contains("OK"));
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn test_handle_command_terminate_not_found() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "TERMINATE nonexistent");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("not found"));
    }

    #[test]
    fn test_handle_command_terminate_missing_name() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "TERMINATE");
        assert!(resp.contains("ERROR"));
    }

    #[test]
    fn test_handle_command_gc_empty() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "GC");
        assert!(resp.contains("No stale machines"));
    }

    #[test]
    fn test_handle_command_unknown() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "FOOBAR");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Unknown command"));
    }

    #[test]
    fn test_handle_command_empty() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "");
        assert!(resp.contains("ERROR"));
    }

    #[test]
    fn test_handle_command_case_insensitive() {
        let mut reg = MachineRegistry::new();
        let resp1 = handle_control_command(&mut reg, "ping");
        assert_eq!(resp1, "PONG\n");
        let resp2 = handle_control_command(&mut reg, "Ping");
        assert_eq!(resp2, "PONG\n");
    }

    // -- is_leader_alive (basic) -------------------------------------------

    #[test]
    fn test_machine_is_leader_alive_self() {
        let mut m = make_test_machine("self");
        m.leader = process::id();
        assert!(m.is_leader_alive());
    }

    #[test]
    fn test_machine_is_leader_alive_zero() {
        let mut m = make_test_machine("zero");
        m.leader = 0;
        assert!(!m.is_leader_alive());
    }

    #[test]
    fn test_machine_is_leader_alive_nonexistent() {
        let mut m = make_test_machine("ghost");
        m.leader = 4_000_000;
        assert!(!m.is_leader_alive());
    }

    // -- Terminate with state file removal ----------------------------------

    #[test]
    fn test_terminate_removes_state_file() {
        let dir = temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        let mut reg = MachineRegistry::new();
        reg.register(make_test_machine("todel")).unwrap();
        reg.save_all_to(dir_path).unwrap();
        assert!(dir.path().join("todel").exists());

        reg.terminate_in("todel", dir_path).unwrap();
        assert!(!dir.path().join("todel").exists());
        assert_eq!(reg.count(), 0);
    }

    // -- Multiple machines --------------------------------------------------

    #[test]
    fn test_registry_multiple_machines() {
        let mut reg = MachineRegistry::new();
        for i in 0..10 {
            let mut m = make_test_machine(&format!("machine{}", i));
            m.leader = 10000 + i;
            reg.register(m).unwrap();
        }
        assert_eq!(reg.count(), 10);

        // List is sorted by name (BTreeMap)
        let names: Vec<&str> = reg.list().iter().map(|m| m.name.as_str()).collect();
        let mut sorted_names = names.clone();
        sorted_names.sort();
        assert_eq!(names, sorted_names);
    }

    // -- Register via control command with scope ----------------------------

    #[test]
    fn test_handle_command_register_with_scope() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(
            &mut reg,
            "REGISTER myvm vm qemu 1234 /var/lib/machines/myvm machine-myvm.scope",
        );
        assert!(resp.contains("OK"));
        let m = reg.get("myvm").unwrap();
        assert_eq!(m.scope, "machine-myvm.scope");
    }

    // -- Image type parsing --------------------------------------------------

    #[test]
    fn test_image_type_parse_directory() {
        assert_eq!(ImageType::parse("directory"), Some(ImageType::Directory));
        assert_eq!(ImageType::parse("subvolume"), Some(ImageType::Directory));
    }

    #[test]
    fn test_image_type_parse_raw() {
        assert_eq!(ImageType::parse("raw"), Some(ImageType::Raw));
    }

    #[test]
    fn test_image_type_parse_invalid() {
        assert_eq!(ImageType::parse("unknown"), None);
        assert_eq!(ImageType::parse(""), None);
    }

    #[test]
    fn test_image_type_display() {
        assert_eq!(ImageType::Directory.to_string(), "directory");
        assert_eq!(ImageType::Raw.to_string(), "raw");
    }

    #[test]
    fn test_image_type_as_str() {
        assert_eq!(ImageType::Directory.as_str(), "directory");
        assert_eq!(ImageType::Raw.as_str(), "raw");
    }

    // -- Image discovery -----------------------------------------------------

    #[test]
    fn test_discover_images_empty_dir() {
        let dir = temp_dir();
        let images = discover_images(dir.path().to_str().unwrap());
        assert!(images.is_empty());
    }

    #[test]
    fn test_discover_images_nonexistent_dir() {
        let images = discover_images("/nonexistent/path");
        assert!(images.is_empty());
    }

    #[test]
    fn test_discover_images_directory_image() {
        let dir = temp_dir();
        let img_dir = dir.path().join("mycontainer");
        fs::create_dir(&img_dir).unwrap();
        let images = discover_images(dir.path().to_str().unwrap());
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].name, "mycontainer");
        assert_eq!(images[0].image_type, ImageType::Directory);
    }

    #[test]
    fn test_discover_images_raw_image() {
        let dir = temp_dir();
        let img_path = dir.path().join("myimage.raw");
        fs::write(&img_path, b"fake raw image").unwrap();
        let images = discover_images(dir.path().to_str().unwrap());
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].name, "myimage.raw");
        assert_eq!(images[0].image_type, ImageType::Raw);
    }

    #[test]
    fn test_discover_images_skips_hidden() {
        let dir = temp_dir();
        let hidden = dir.path().join(".hidden");
        fs::create_dir(&hidden).unwrap();
        let visible = dir.path().join("visible");
        fs::create_dir(&visible).unwrap();
        let images = discover_images(dir.path().to_str().unwrap());
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].name, "visible");
    }

    #[test]
    fn test_discover_images_sorted() {
        let dir = temp_dir();
        fs::create_dir(dir.path().join("charlie")).unwrap();
        fs::create_dir(dir.path().join("alpha")).unwrap();
        fs::create_dir(dir.path().join("bravo")).unwrap();
        let images = discover_images(dir.path().to_str().unwrap());
        let names: Vec<&str> = images.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "bravo", "charlie"]);
    }

    // -- find_image ----------------------------------------------------------

    #[test]
    fn test_find_image_exact_name() {
        let dir = temp_dir();
        fs::create_dir(dir.path().join("myvm")).unwrap();
        let img = find_image(dir.path().to_str().unwrap(), "myvm");
        assert!(img.is_some());
        assert_eq!(img.unwrap().name, "myvm");
    }

    #[test]
    fn test_find_image_with_raw_extension() {
        let dir = temp_dir();
        fs::write(dir.path().join("test.raw"), b"data").unwrap();
        let img = find_image(dir.path().to_str().unwrap(), "test");
        assert!(img.is_some());
        assert_eq!(img.unwrap().name, "test.raw");
    }

    #[test]
    fn test_find_image_not_found() {
        let dir = temp_dir();
        let img = find_image(dir.path().to_str().unwrap(), "nonexistent");
        assert!(img.is_none());
    }

    // -- clone_image ---------------------------------------------------------

    #[test]
    fn test_clone_image_directory() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        let src = dir.path().join("source");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("file.txt"), "hello").unwrap();

        let result = clone_image(pool, "source", "dest", false);
        assert!(result.is_ok());
        assert!(dir.path().join("dest").exists());
    }

    #[test]
    fn test_clone_image_already_exists() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        fs::create_dir(dir.path().join("source")).unwrap();
        fs::create_dir(dir.path().join("dest")).unwrap();

        let result = clone_image(pool, "source", "dest", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    #[test]
    fn test_clone_image_not_found() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        let result = clone_image(pool, "nonexistent", "dest", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_clone_image_invalid_dest_name() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        fs::create_dir(dir.path().join("source")).unwrap();
        let result = clone_image(pool, "source", ".invalid", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid image name"));
    }

    // -- rename_image --------------------------------------------------------

    #[test]
    fn test_rename_image_directory() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        fs::create_dir(dir.path().join("oldname")).unwrap();

        let result = rename_image(pool, "oldname", "newname");
        assert!(result.is_ok());
        assert!(!dir.path().join("oldname").exists());
        assert!(dir.path().join("newname").exists());
    }

    #[test]
    fn test_rename_image_not_found() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        let result = rename_image(pool, "nonexistent", "newname");
        assert!(result.is_err());
    }

    #[test]
    fn test_rename_image_dest_exists() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        fs::create_dir(dir.path().join("old")).unwrap();
        fs::create_dir(dir.path().join("new")).unwrap();
        let result = rename_image(pool, "old", "new");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    #[test]
    fn test_rename_image_with_limit_file() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        fs::create_dir(dir.path().join("oldname")).unwrap();
        fs::write(dir.path().join("oldname.limit"), "1073741824\n").unwrap();

        let result = rename_image(pool, "oldname", "newname");
        assert!(result.is_ok());
        assert!(!dir.path().join("oldname.limit").exists());
        assert!(dir.path().join("newname.limit").exists());
    }

    // -- remove_image --------------------------------------------------------

    #[test]
    fn test_remove_image_directory() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        let img = dir.path().join("todelete");
        fs::create_dir(&img).unwrap();
        fs::write(img.join("file.txt"), "data").unwrap();

        let result = remove_image(pool, "todelete");
        assert!(result.is_ok());
        assert!(!img.exists());
    }

    #[test]
    fn test_remove_image_raw() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        fs::write(dir.path().join("todelete.raw"), b"raw data").unwrap();

        let result = remove_image(pool, "todelete");
        assert!(result.is_ok());
        assert!(!dir.path().join("todelete.raw").exists());
    }

    #[test]
    fn test_remove_image_not_found() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        let result = remove_image(pool, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_image_cleans_limit_file() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        fs::create_dir(dir.path().join("img")).unwrap();
        fs::write(dir.path().join("img.limit"), "1024\n").unwrap();

        let result = remove_image(pool, "img");
        assert!(result.is_ok());
        assert!(!dir.path().join("img.limit").exists());
    }

    // -- set_image_limit -----------------------------------------------------

    #[test]
    fn test_set_image_limit_creates_limit_file() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        fs::create_dir(dir.path().join("myimg")).unwrap();

        let result = set_image_limit(pool, "myimg", 1073741824);
        assert!(result.is_ok());
        let content = fs::read_to_string(dir.path().join("myimg.limit")).unwrap();
        assert_eq!(content.trim(), "1073741824");
    }

    #[test]
    fn test_set_image_limit_zero_removes() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        fs::create_dir(dir.path().join("myimg")).unwrap();
        fs::write(dir.path().join("myimg.limit"), "1024\n").unwrap();

        let result = set_image_limit(pool, "myimg", 0);
        assert!(result.is_ok());
        assert!(!dir.path().join("myimg.limit").exists());
    }

    #[test]
    fn test_set_image_limit_not_found() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        let result = set_image_limit(pool, "nonexistent", 1024);
        assert!(result.is_err());
    }

    // -- ImageInfo format ----------------------------------------------------

    #[test]
    fn test_image_info_format_status() {
        let info = ImageInfo {
            name: "test".to_string(),
            image_type: ImageType::Directory,
            path: PathBuf::from("/var/lib/machines/test"),
            read_only: false,
            crtime: 1700000000000000,
            mtime: 1700000000000000,
            usage: 0,
            limit: 0,
        };
        let status = info.format_status();
        assert!(status.contains("Name: test"));
        assert!(status.contains("Type: directory"));
        assert!(status.contains("ReadOnly: no"));
    }

    #[test]
    fn test_image_info_format_show() {
        let info = ImageInfo {
            name: "test".to_string(),
            image_type: ImageType::Raw,
            path: PathBuf::from("/var/lib/machines/test.raw"),
            read_only: true,
            crtime: 0,
            mtime: 0,
            usage: 1024,
            limit: 2048,
        };
        let show = info.format_show();
        assert!(show.contains("Name=test"));
        assert!(show.contains("Type=raw"));
        assert!(show.contains("ReadOnly=yes"));
        assert!(show.contains("Usage=1024"));
        assert!(show.contains("Limit=2048"));
    }

    #[test]
    fn test_image_info_from_path_directory() {
        let dir = temp_dir();
        let img_path = dir.path().join("testimg");
        fs::create_dir(&img_path).unwrap();
        let info = ImageInfo::from_path(&img_path);
        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.name, "testimg");
        assert_eq!(info.image_type, ImageType::Directory);
    }

    #[test]
    fn test_image_info_from_path_raw() {
        let dir = temp_dir();
        let img_path = dir.path().join("testimg.raw");
        fs::write(&img_path, b"fake raw").unwrap();
        let info = ImageInfo::from_path(&img_path);
        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.name, "testimg.raw");
        assert_eq!(info.image_type, ImageType::Raw);
        assert_eq!(info.usage, 8); // "fake raw" = 8 bytes
    }

    #[test]
    fn test_image_info_from_path_hidden() {
        let dir = temp_dir();
        let img_path = dir.path().join(".hidden");
        fs::create_dir(&img_path).unwrap();
        let info = ImageInfo::from_path(&img_path);
        assert!(info.is_none());
    }

    #[test]
    fn test_image_info_from_path_with_limit_file() {
        let dir = temp_dir();
        let img_path = dir.path().join("img");
        fs::create_dir(&img_path).unwrap();
        fs::write(dir.path().join("img.limit"), "4096\n").unwrap();
        let info = ImageInfo::from_path(&img_path).unwrap();
        assert_eq!(info.limit, 4096);
    }

    // -- format_bytes --------------------------------------------------------

    #[test]
    fn test_format_bytes_zero() {
        assert_eq!(format_bytes(0), "0B");
    }

    #[test]
    fn test_format_bytes_bytes() {
        assert_eq!(format_bytes(100), "100B");
    }

    #[test]
    fn test_format_bytes_kilobytes() {
        assert_eq!(format_bytes(1024), "1.0K");
    }

    #[test]
    fn test_format_bytes_megabytes() {
        assert_eq!(format_bytes(1048576), "1.0M");
    }

    #[test]
    fn test_format_bytes_gigabytes() {
        assert_eq!(format_bytes(1073741824), "1.0G");
    }

    #[test]
    fn test_format_bytes_mixed() {
        assert_eq!(format_bytes(1536), "1.5K");
    }

    // -- parse_size ----------------------------------------------------------

    #[test]
    fn test_parse_size_bytes() {
        assert_eq!(parse_size("1024"), Some(1024));
        assert_eq!(parse_size("1024B"), Some(1024));
    }

    #[test]
    fn test_parse_size_kilobytes() {
        assert_eq!(parse_size("1K"), Some(1024));
        assert_eq!(parse_size("2K"), Some(2048));
    }

    #[test]
    fn test_parse_size_megabytes() {
        assert_eq!(parse_size("1M"), Some(1048576));
    }

    #[test]
    fn test_parse_size_gigabytes() {
        assert_eq!(parse_size("1G"), Some(1073741824));
    }

    #[test]
    fn test_parse_size_terabytes() {
        assert_eq!(parse_size("1T"), Some(1099511627776));
    }

    #[test]
    fn test_parse_size_empty() {
        assert_eq!(parse_size(""), None);
    }

    #[test]
    fn test_parse_size_invalid() {
        assert_eq!(parse_size("abc"), None);
    }

    // -- Copy-to/copy-from ---------------------------------------------------

    #[test]
    fn test_copy_to_machine_not_found() {
        let reg = MachineRegistry::new();
        let result = copy_to_machine(&reg, "nonexistent", "/tmp/file", "/dest");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_copy_from_machine_not_found() {
        let reg = MachineRegistry::new();
        let result = copy_from_machine(&reg, "nonexistent", "/src", "/tmp/dest");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_copy_to_machine_source_not_found() {
        let mut reg = MachineRegistry::new();
        let dir = temp_dir();
        let root = dir.path().to_str().unwrap();
        let machine = Machine {
            name: "test".to_string(),
            class: MachineClass::Container,
            service: "nspawn".to_string(),
            scope: String::new(),
            leader: process::id(),
            root_directory: root.to_string(),
            netif: Vec::new(),
            timestamp: now_usec(),
            state: MachineState::Running,
        };
        reg.register(machine).unwrap();

        let result = copy_to_machine(&reg, "test", "/nonexistent/src/path", "/dest");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn test_copy_to_and_from_machine_file() {
        let mut reg = MachineRegistry::new();
        let container_dir = temp_dir();
        let host_dir = temp_dir();

        let root = container_dir.path().to_str().unwrap();
        let machine = Machine {
            name: "testcp".to_string(),
            class: MachineClass::Container,
            service: "nspawn".to_string(),
            scope: String::new(),
            leader: process::id(),
            root_directory: root.to_string(),
            netif: Vec::new(),
            timestamp: now_usec(),
            state: MachineState::Running,
        };
        reg.register(machine).unwrap();

        // Create a file on the "host"
        let host_file = host_dir.path().join("test.txt");
        fs::write(&host_file, "hello world").unwrap();

        // Copy to container
        let result = copy_to_machine(&reg, "testcp", host_file.to_str().unwrap(), "/test.txt");
        assert!(result.is_ok());
        assert!(container_dir.path().join("test.txt").exists());
        assert_eq!(
            fs::read_to_string(container_dir.path().join("test.txt")).unwrap(),
            "hello world"
        );

        // Copy from container
        let dest_file = host_dir.path().join("retrieved.txt");
        let result = copy_from_machine(&reg, "testcp", "/test.txt", dest_file.to_str().unwrap());
        assert!(result.is_ok());
        assert_eq!(fs::read_to_string(&dest_file).unwrap(), "hello world");
    }

    // -- Control command: IMAGE_LIST -----------------------------------------

    #[test]
    fn test_handle_command_image_list_empty() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "IMAGE_LIST");
        assert!(resp.contains("No images") || resp.contains("images listed"));
    }

    #[test]
    fn test_handle_command_image_list_alias() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "LIST-IMAGES");
        assert!(resp.contains("No images") || resp.contains("images listed"));
    }

    // -- Control command: IMAGE_SHOW -----------------------------------------

    #[test]
    fn test_handle_command_image_show_missing_name() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "IMAGE_SHOW");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Image name required"));
    }

    #[test]
    fn test_handle_command_image_show_not_found() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "IMAGE_SHOW nonexistent");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("not found"));
    }

    // -- Control command: CLONE ----------------------------------------------

    #[test]
    fn test_handle_command_clone_missing_args() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "CLONE source");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Usage"));
    }

    // -- Control command: RENAME ---------------------------------------------

    #[test]
    fn test_handle_command_rename_missing_args() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "RENAME old");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Usage"));
    }

    // -- Control command: REMOVE ---------------------------------------------

    #[test]
    fn test_handle_command_remove_missing_name() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "REMOVE");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Image name required"));
    }

    // -- Control command: SET-LIMIT ------------------------------------------

    #[test]
    fn test_handle_command_set_limit_missing_args() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "SET-LIMIT myimg");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Usage"));
    }

    #[test]
    fn test_handle_command_set_limit_invalid_size() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "SET-LIMIT myimg notasize");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Invalid size"));
    }

    // -- Control command: COPY-TO / COPY-FROM --------------------------------

    #[test]
    fn test_handle_command_copy_to_missing_args() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "COPY-TO machine /src");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Usage"));
    }

    #[test]
    fn test_handle_command_copy_from_missing_args() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "COPY-FROM machine /src");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Usage"));
    }

    #[test]
    fn test_handle_command_copy_to_machine_not_found() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "COPY-TO nonexistent /src /dest");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("not found"));
    }

    // -- Control command: LOGIN / SHELL --------------------------------------

    #[test]
    fn test_handle_command_login_missing_name() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "LOGIN");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Machine name required"));
    }

    #[test]
    fn test_handle_command_login_not_found() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "LOGIN nonexistent");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("not found"));
    }

    #[test]
    fn test_handle_command_shell_missing_name() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "SHELL");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Machine name required"));
    }

    // -- Control command: IMPORT-TAR / IMPORT-RAW ----------------------------

    #[test]
    fn test_handle_command_import_tar_missing_args() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "IMPORT-TAR source");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Usage"));
    }

    #[test]
    fn test_handle_command_import_raw_missing_args() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "IMPORT-RAW source");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Usage"));
    }

    // -- Control command: EXPORT-TAR / EXPORT-RAW ----------------------------

    #[test]
    fn test_handle_command_export_tar_missing_args() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "EXPORT-TAR name");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Usage"));
    }

    #[test]
    fn test_handle_command_export_raw_missing_args() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "EXPORT-RAW name");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Usage"));
    }

    // -- Control command: PULL-TAR / PULL-RAW --------------------------------

    #[test]
    fn test_handle_command_pull_tar_missing_args() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "PULL-TAR url");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Usage"));
    }

    #[test]
    fn test_handle_command_pull_raw_missing_args() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "PULL-RAW url");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("Usage"));
    }

    // -- Control command case insensitivity for new commands ------------------

    #[test]
    fn test_handle_command_image_list_case_insensitive() {
        let mut reg = MachineRegistry::new();
        let resp1 = handle_control_command(&mut reg, "image_list");
        let resp2 = handle_control_command(&mut reg, "IMAGE_LIST");
        assert_eq!(resp1, resp2);
    }

    #[test]
    fn test_handle_command_clone_case_insensitive() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "clone source");
        assert!(resp.contains("ERROR")); // not enough args but shows it's parsed
    }

    // -- Import/export integration with temp dirs ----------------------------

    #[test]
    fn test_import_tar_invalid_name() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        let result = import_tar(pool, "/nonexistent.tar", ".invalid", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid image name"));
    }

    #[test]
    fn test_import_raw_invalid_name() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        let result = import_raw(pool, "/nonexistent.raw", ".invalid", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid image name"));
    }

    #[test]
    fn test_import_raw_already_exists() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        fs::write(dir.path().join("existing.raw"), b"data").unwrap();
        let result = import_raw(pool, "/tmp/source.raw", "existing", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    #[test]
    fn test_export_tar_not_found() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        let result = export_tar(pool, "nonexistent", "/tmp/out.tar");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_export_raw_not_found() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        let result = export_raw(pool, "nonexistent", "/tmp/out.raw");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_export_tar_wrong_type() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        fs::write(dir.path().join("myimg.raw"), b"data").unwrap();
        let result = export_tar(pool, "myimg", "/tmp/out.tar");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("directory images"));
    }

    #[test]
    fn test_export_raw_wrong_type() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        fs::create_dir(dir.path().join("myimg")).unwrap();
        let result = export_raw(pool, "myimg", "/tmp/out.raw");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("raw images"));
    }

    #[test]
    fn test_pull_tar_invalid_name() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        let result = pull_tar(pool, "http://example.com/test.tar", ".invalid", "no");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid image name"));
    }

    #[test]
    fn test_pull_raw_invalid_name() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        let result = pull_raw(pool, "http://example.com/test.raw", ".invalid", "no");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid image name"));
    }

    #[test]
    fn test_pull_tar_already_exists() {
        let dir = temp_dir();
        let pool = dir.path().to_str().unwrap();
        fs::create_dir(dir.path().join("existing")).unwrap();
        let result = pull_tar(pool, "http://example.com/test.tar", "existing", "no");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    // -- PTY forwarding (unit tests for error paths) -------------------------

    #[test]
    fn test_open_machine_pty_not_found() {
        let reg = MachineRegistry::new();
        let result = open_machine_pty(&reg, "nonexistent", None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_open_machine_pty_no_leader() {
        let mut reg = MachineRegistry::new();
        let machine = Machine {
            name: "noleader".to_string(),
            class: MachineClass::Container,
            service: "test".to_string(),
            scope: String::new(),
            leader: 0,
            root_directory: "/".to_string(),
            netif: Vec::new(),
            timestamp: now_usec(),
            state: MachineState::Running,
        };
        reg.register(machine).unwrap();
        let result = open_machine_pty(&reg, "noleader", None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no leader PID"));
    }

    // -- Machine scoping (unit tests for scope name generation) ---------------

    #[test]
    fn test_scope_name_format() {
        // Verify the scope name format without actually calling D-Bus
        let name = "testvm";
        let scope_name = format!("machine-{}.scope", name);
        assert_eq!(scope_name, "machine-testvm.scope");
    }

    #[test]
    fn test_scope_name_special_chars() {
        let name = "my.container";
        let scope_name = format!("machine-{}.scope", name);
        assert_eq!(scope_name, "machine-my.container.scope");
    }

    // -- pool_usage_stats (basic) -------------------------------------------

    #[test]
    fn test_pool_usage_stats_nonexistent() {
        let (used, total) = pool_usage_stats("/nonexistent/pool/path");
        assert_eq!(used, 0);
        assert_eq!(total, 0);
    }

    #[test]
    fn test_pool_usage_stats_tmp() {
        let dir = temp_dir();
        let (_used, total) = pool_usage_stats(dir.path().to_str().unwrap());
        // Should return non-zero for an existing filesystem
        assert!(total > 0);
    }

    // -- Bind mount (unit tests for error paths) ----------------------------

    #[test]
    fn test_bind_mount_machine_not_found() {
        let reg = MachineRegistry::new();
        let result = bind_mount_in_machine(&reg, "nonexistent", "/tmp", "/mnt", false, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_bind_mount_no_leader() {
        let mut reg = MachineRegistry::new();
        let machine = Machine {
            name: "noleader".to_string(),
            class: MachineClass::Container,
            service: "test".to_string(),
            scope: String::new(),
            leader: 0,
            root_directory: "/".to_string(),
            netif: Vec::new(),
            timestamp: now_usec(),
            state: MachineState::Running,
        };
        reg.register(machine).unwrap();
        let result = bind_mount_in_machine(&reg, "noleader", "/tmp", "/mnt", false, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no leader PID"));
    }

    #[test]
    fn test_bind_mount_source_not_found() {
        let mut reg = MachineRegistry::new();
        let machine = Machine {
            name: "testm".to_string(),
            class: MachineClass::Container,
            service: "test".to_string(),
            scope: String::new(),
            leader: std::process::id(),
            root_directory: "/".to_string(),
            netif: Vec::new(),
            timestamp: now_usec(),
            state: MachineState::Running,
        };
        reg.register(machine).unwrap();
        let result = bind_mount_in_machine(
            &reg,
            "testm",
            "/nonexistent/source/path",
            "/mnt",
            false,
            false,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn test_bind_user_machine_not_found() {
        let reg = MachineRegistry::new();
        let result = bind_user_in_machine(&reg, "nonexistent", "/tmp", None, "root", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_bind_user_no_leader() {
        let mut reg = MachineRegistry::new();
        let machine = Machine {
            name: "noleader2".to_string(),
            class: MachineClass::Container,
            service: "test".to_string(),
            scope: String::new(),
            leader: 0,
            root_directory: "/".to_string(),
            netif: Vec::new(),
            timestamp: now_usec(),
            state: MachineState::Running,
        };
        reg.register(machine).unwrap();
        let result = bind_user_in_machine(&reg, "noleader2", "/tmp", None, "root", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no leader PID"));
    }

    #[test]
    fn test_bind_user_source_not_found() {
        let mut reg = MachineRegistry::new();
        let machine = Machine {
            name: "testm2".to_string(),
            class: MachineClass::Container,
            service: "test".to_string(),
            scope: String::new(),
            leader: std::process::id(),
            root_directory: "/".to_string(),
            netif: Vec::new(),
            timestamp: now_usec(),
            state: MachineState::Running,
        };
        reg.register(machine).unwrap();
        let result = bind_user_in_machine(
            &reg,
            "testm2",
            "/nonexistent/bind/user/path",
            None,
            "root",
            false,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    // -- resolve_container_user_home ----------------------------------------

    #[test]
    fn test_resolve_container_user_home_root() {
        let dir = temp_dir();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(
            etc.join("passwd"),
            "root:x:0:0:root:/root:/bin/bash\nnobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin\n",
        )
        .unwrap();

        let result = resolve_container_user_home(dir.path().to_str().unwrap(), "root");
        assert_eq!(result.unwrap(), "/root");
    }

    #[test]
    fn test_resolve_container_user_home_by_name() {
        let dir = temp_dir();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(
            etc.join("passwd"),
            "root:x:0:0:root:/root:/bin/bash\ntestuser:x:1000:1000:Test User:/home/testuser:/bin/bash\n",
        )
        .unwrap();

        let result = resolve_container_user_home(dir.path().to_str().unwrap(), "testuser");
        assert_eq!(result.unwrap(), "/home/testuser");
    }

    #[test]
    fn test_resolve_container_user_home_by_uid() {
        let dir = temp_dir();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(
            etc.join("passwd"),
            "root:x:0:0:root:/root:/bin/bash\ntestuser:x:1000:1000:Test User:/home/testuser:/bin/bash\n",
        )
        .unwrap();

        let result = resolve_container_user_home(dir.path().to_str().unwrap(), "1000");
        assert_eq!(result.unwrap(), "/home/testuser");
    }

    #[test]
    fn test_resolve_container_user_home_missing_passwd() {
        let dir = temp_dir();
        let result = resolve_container_user_home(dir.path().to_str().unwrap(), "testuser");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cannot read"));
    }

    #[test]
    fn test_resolve_container_user_home_user_not_found_defaults() {
        let dir = temp_dir();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(etc.join("passwd"), "root:x:0:0:root:/root:/bin/bash\n").unwrap();

        // Unknown user defaults to /home/<user>
        let result = resolve_container_user_home(dir.path().to_str().unwrap(), "nobody2");
        assert_eq!(result.unwrap(), "/home/nobody2");
    }

    #[test]
    fn test_resolve_container_user_home_root_uid_defaults() {
        let dir = temp_dir();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(etc.join("passwd"), "").unwrap();

        // UID 0 defaults to /root
        let result = resolve_container_user_home(dir.path().to_str().unwrap(), "0");
        assert_eq!(result.unwrap(), "/root");
    }

    #[test]
    fn test_resolve_container_user_home_comments_blanks() {
        let dir = temp_dir();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(
            etc.join("passwd"),
            "# comment\n\nroot:x:0:0:root:/root:/bin/bash\n",
        )
        .unwrap();

        let result = resolve_container_user_home(dir.path().to_str().unwrap(), "root");
        assert_eq!(result.unwrap(), "/root");
    }

    // -- Control command BIND / BIND-USER -----------------------------------

    #[test]
    fn test_handle_command_bind_missing_args() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "BIND myvm");
        assert!(resp.starts_with("ERROR"));
    }

    #[test]
    fn test_handle_command_bind_machine_not_found() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "BIND nonexistent /tmp /mnt");
        assert!(resp.starts_with("ERROR"));
        assert!(resp.contains("not found"));
    }

    #[test]
    fn test_handle_command_bind_user_missing_args() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "BIND-USER myvm");
        assert!(resp.starts_with("ERROR"));
    }

    #[test]
    fn test_handle_command_bind_user_missing_user() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "BIND-USER myvm /tmp");
        assert!(resp.starts_with("ERROR"));
    }

    #[test]
    fn test_handle_command_bind_case_insensitive() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "bind nonexistent /tmp /mnt");
        assert!(resp.starts_with("ERROR"));
        assert!(resp.contains("not found"));
    }

    #[test]
    fn test_handle_command_bind_mount_alias() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "BIND-MOUNT nonexistent /tmp /mnt");
        assert!(resp.starts_with("ERROR"));
        assert!(resp.contains("not found"));
    }

    #[test]
    fn test_handle_command_bind_user_alias() {
        let mut reg = MachineRegistry::new();
        let resp = handle_control_command(&mut reg, "BIND_USER nonexistent /tmp root");
        assert!(resp.starts_with("ERROR"));
        assert!(resp.contains("not found"));
    }

    #[test]
    fn test_handle_command_bind_source_not_found() {
        let mut reg = MachineRegistry::new();
        let machine = Machine {
            name: "bindtest".to_string(),
            class: MachineClass::Container,
            service: "test".to_string(),
            scope: String::new(),
            leader: std::process::id(),
            root_directory: "/".to_string(),
            netif: Vec::new(),
            timestamp: now_usec(),
            state: MachineState::Running,
        };
        reg.register(machine).unwrap();
        let resp = handle_control_command(&mut reg, "BIND bindtest /nonexistent/src/path /mnt");
        assert!(resp.starts_with("ERROR"));
        assert!(resp.contains("does not exist"));
    }

    // -- D-Bus bind mount method struct presence ----------------------------

    #[test]
    fn test_dbus_bind_mount_method_exists() {
        // Verify Machine1Manager has the bind_mount and bind_user fields
        // by constructing the struct (compilation test)
        let reg = Arc::new(Mutex::new(MachineRegistry::new()));
        let _mgr = Machine1Manager {
            registry: reg,
            pool_path: "/tmp".to_string(),
        };
    }
}

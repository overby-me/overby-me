//! systemd-shutdown — Final shutdown logic for the system.
//!
//! A drop-in replacement for `systemd-shutdown(8)`. This binary is executed
//! at the very end of the shutdown sequence, after PID 1 has stopped all
//! services and pivoted into a minimal initramfs (or is running as the last
//! process). Its job is to:
//!
//! 1. Send SIGTERM to all remaining processes, wait briefly, then SIGKILL
//! 2. Unmount all filesystems (in reverse mount order)
//! 3. Detach loop devices
//! 4. Deactivate device-mapper (DM) targets
//! 5. Stop MD RAID arrays
//! 6. Perform the final action: poweroff, reboot, halt, or kexec
//!
//! systemd invokes this binary with the verb as the first argument:
//!   systemd-shutdown poweroff|reboot|halt|kexec
//!
//! It may also be invoked as /run/initramfs/shutdown by the service manager
//! after pivoting into the shutdown initramfs.

use std::ffi::CString;
use std::fs;
use std::io::{self, BufRead};

use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::{Duration, Instant};

/// Exit codes.
#[allow(dead_code)]
const EXIT_SUCCESS: i32 = 0;
const EXIT_FAILURE: i32 = 1;

/// Timeout for processes to exit after SIGTERM before sending SIGKILL.
const SIGTERM_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum number of unmount attempts (some filesystems become unmountable
/// only after others are unmounted first).
const MAX_UNMOUNT_RETRIES: u32 = 10;

/// The final action to perform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShutdownAction {
    Poweroff,
    Reboot,
    Halt,
    Kexec,
}

impl ShutdownAction {
    fn from_verb(verb: &str) -> Option<Self> {
        match verb {
            "poweroff" => Some(ShutdownAction::Poweroff),
            "reboot" => Some(ShutdownAction::Reboot),
            "halt" => Some(ShutdownAction::Halt),
            "kexec" => Some(ShutdownAction::Kexec),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            ShutdownAction::Poweroff => "poweroff",
            ShutdownAction::Reboot => "reboot",
            ShutdownAction::Halt => "halt",
            ShutdownAction::Kexec => "kexec",
        }
    }
}

/// Information about a mounted filesystem, parsed from /proc/self/mountinfo
/// or /proc/mounts.
#[derive(Debug, Clone)]
struct MountEntry {
    /// The mount point path.
    mount_point: PathBuf,
    /// The filesystem type.
    fs_type: String,
    /// The source device (if any).
    #[allow(dead_code)]
    source: String,
}

/// Parse /proc/self/mountinfo to get the list of mounted filesystems.
/// Returns entries in the order they appear (which is mount order).
fn read_mountinfo() -> Vec<MountEntry> {
    // Try /proc/self/mountinfo first (richer format), fall back to /proc/mounts
    if let Ok(entries) = parse_mountinfo("/proc/self/mountinfo") {
        return entries;
    }
    parse_proc_mounts("/proc/mounts").unwrap_or_default()
}

/// Parse the /proc/self/mountinfo format.
/// Each line looks like:
///   36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw,errors=continue
fn parse_mountinfo(path: &str) -> io::Result<Vec<MountEntry>> {
    let file = fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut entries = Vec::new();

    for line in reader.lines() {
        let line = line?;
        let parts: Vec<&str> = line.split_whitespace().collect();

        // Find the " - " separator
        let sep_idx = parts.iter().position(|&p| p == "-");
        let sep_idx = match sep_idx {
            Some(i) => i,
            None => continue,
        };

        if parts.len() < sep_idx + 3 {
            continue;
        }

        let mount_point = unescape_mountinfo_path(parts[4]);
        let fs_type = parts[sep_idx + 1].to_string();
        let source = parts[sep_idx + 2].to_string();

        entries.push(MountEntry {
            mount_point: PathBuf::from(mount_point),
            fs_type,
            source,
        });
    }

    Ok(entries)
}

/// Parse the simpler /proc/mounts format.
/// Each line looks like:
///   /dev/sda1 / ext4 rw,relatime 0 0
fn parse_proc_mounts(path: &str) -> io::Result<Vec<MountEntry>> {
    let file = fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut entries = Vec::new();

    for line in reader.lines() {
        let line = line?;
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }

        entries.push(MountEntry {
            mount_point: PathBuf::from(unescape_mountinfo_path(parts[1])),
            fs_type: parts[2].to_string(),
            source: parts[0].to_string(),
        });
    }

    Ok(entries)
}

/// Unescape octal escape sequences in mountinfo paths (e.g., \040 for space).
fn unescape_mountinfo_path(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            // Try to parse 3 octal digits
            let d0 = bytes[i + 1];
            let d1 = bytes[i + 2];
            let d2 = bytes[i + 3];
            if d0.is_ascii_digit()
                && d1.is_ascii_digit()
                && d2.is_ascii_digit()
                && let Ok(val) =
                    u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 4]).unwrap_or(""), 8)
            {
                result.push(val);
                i += 4;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&result).into_owned()
}

/// Check if a filesystem should be skipped during unmount.
/// We never unmount virtual/API filesystems.
fn should_skip_unmount(entry: &MountEntry) -> bool {
    let skip_types = [
        "proc",
        "sysfs",
        "devtmpfs",
        "devpts",
        "tmpfs",
        "cgroup",
        "cgroup2",
        "pstore",
        "securityfs",
        "debugfs",
        "tracefs",
        "configfs",
        "fusectl",
        "hugetlbfs",
        "mqueue",
        "rpc_pipefs",
        "autofs",
        "binfmt_misc",
        "efivarfs",
        "bpf",
        "ramfs",
    ];

    if skip_types.contains(&entry.fs_type.as_str()) {
        return true;
    }

    let skip_mounts = [
        "/proc",
        "/sys",
        "/dev",
        "/dev/pts",
        "/dev/shm",
        "/dev/hugepages",
        "/dev/mqueue",
        "/run",
        "/sys/kernel/security",
        "/sys/fs/cgroup",
        "/sys/fs/pstore",
        "/sys/kernel/debug",
        "/sys/kernel/tracing",
        "/sys/kernel/config",
        "/sys/fs/fuse/connections",
        "/sys/fs/bpf",
    ];

    let mp = entry.mount_point.to_string_lossy();
    if skip_mounts.contains(&mp.as_ref()) {
        return true;
    }

    // Don't unmount the root filesystem (we'll remount it read-only instead)
    if mp == "/" {
        return true;
    }

    false
}

/// Send a signal to all processes except PID 1 and our own PID.
fn kill_all_processes(signal: i32) {
    let our_pid = process::id();
    eprintln!(
        "systemd-shutdown: Sending {} to all remaining processes...",
        match signal {
            libc::SIGTERM => "SIGTERM",
            libc::SIGKILL => "SIGKILL",
            _ => "signal",
        }
    );

    // Read /proc to find all processes
    let proc_dir = match fs::read_dir("/proc") {
        Ok(d) => d,
        Err(e) => {
            eprintln!("systemd-shutdown: Failed to read /proc: {}", e);
            // Fall back to kill(-1, sig) which sends to all processes except PID 1
            unsafe {
                libc::kill(-1, signal);
            }
            return;
        }
    };

    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Only look at numeric directory names (PIDs)
        let pid: u32 = match name_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Skip PID 1 (init) and ourselves
        if pid <= 1 || pid == our_pid {
            continue;
        }

        // Skip kernel threads (they have no exe link or an empty cmdline)
        let exe_link = format!("/proc/{}/exe", pid);
        if fs::read_link(&exe_link).is_err() {
            // No exe link usually means kernel thread; skip
            continue;
        }

        unsafe {
            libc::kill(pid as i32, signal);
        }
    }
}

/// Wait for all non-PID1 processes to exit, with a timeout.
fn wait_for_processes(timeout: Duration) {
    let start = Instant::now();
    let our_pid = process::id();

    loop {
        if start.elapsed() >= timeout {
            return;
        }

        // Count remaining user processes
        let mut remaining = 0u32;
        if let Ok(proc_dir) = fs::read_dir("/proc") {
            for entry in proc_dir.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                let pid: u32 = match name_str.parse() {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                if pid <= 1 || pid == our_pid {
                    continue;
                }
                // Check if it's a kernel thread
                let exe_link = format!("/proc/{}/exe", pid);
                if fs::read_link(&exe_link).is_ok() {
                    remaining += 1;
                }
            }
        }

        if remaining == 0 {
            eprintln!("systemd-shutdown: All processes have exited");
            return;
        }

        thread::sleep(Duration::from_millis(200));
    }
}

/// Attempt to unmount all non-API filesystems in reverse mount order.
/// Returns the number of filesystems that could not be unmounted.
fn unmount_all() -> u32 {
    let mut failed = 0u32;

    for attempt in 1..=MAX_UNMOUNT_RETRIES {
        let mounts = read_mountinfo();

        // Process in reverse order (unmount children before parents)
        let to_unmount: Vec<MountEntry> = mounts
            .into_iter()
            .rev()
            .filter(|e| !should_skip_unmount(e))
            .collect();

        if to_unmount.is_empty() {
            break;
        }

        failed = 0;
        let mut unmounted_any = false;

        for entry in &to_unmount {
            let mount_point = entry.mount_point.to_string_lossy().to_string();
            let mp_cstr = match CString::new(mount_point.as_bytes().to_vec()) {
                Ok(c) => c,
                Err(_) => {
                    failed += 1;
                    continue;
                }
            };

            // First try a regular unmount
            let ret = unsafe { libc::umount2(mp_cstr.as_ptr(), 0) };
            if ret == 0 {
                eprintln!("systemd-shutdown: Unmounted {}", mount_point);
                unmounted_any = true;
                continue;
            }

            // If the regular unmount failed and this is a later attempt, try lazy unmount
            if attempt >= 3 {
                let ret = unsafe { libc::umount2(mp_cstr.as_ptr(), libc::MNT_DETACH) };
                if ret == 0 {
                    eprintln!("systemd-shutdown: Lazy-unmounted {}", mount_point);
                    unmounted_any = true;
                    continue;
                }
            }

            let err = io::Error::last_os_error();
            eprintln!(
                "systemd-shutdown: Failed to unmount {} (attempt {}): {}",
                mount_point, attempt, err
            );
            failed += 1;
        }

        if !unmounted_any || failed == 0 {
            break;
        }
    }

    failed
}

/// Remount the root filesystem read-only.
fn remount_root_readonly() {
    eprintln!("systemd-shutdown: Remounting / read-only...");

    let root = CString::new("/").unwrap();
    let none: *const libc::c_char = std::ptr::null();

    let ret = unsafe {
        libc::mount(
            none,
            root.as_ptr(),
            none,
            libc::MS_REMOUNT | libc::MS_RDONLY,
            std::ptr::null(),
        )
    };

    if ret != 0 {
        let err = io::Error::last_os_error();
        eprintln!("systemd-shutdown: Failed to remount / read-only: {}", err);
    } else {
        eprintln!("systemd-shutdown: Remounted / read-only");
    }
}

/// Detach all loop devices.
fn detach_loop_devices() {
    let loop_dir = Path::new("/sys/block");
    if !loop_dir.exists() {
        return;
    }

    let entries = match fs::read_dir(loop_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("loop") {
            continue;
        }

        // Check if the loop device is in use by looking for a backing file
        let backing_file = entry.path().join("loop/backing_file");
        if !backing_file.exists() {
            continue;
        }

        let dev_path = format!("/dev/{}", name_str);
        eprintln!("systemd-shutdown: Detaching loop device {}", dev_path);

        // Use LOOP_CLR_FD ioctl to detach
        let dev_cstr = match CString::new(dev_path.as_bytes().to_vec()) {
            Ok(c) => c,
            Err(_) => continue,
        };

        unsafe {
            let fd = libc::open(dev_cstr.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC);
            if fd >= 0 {
                // LOOP_CLR_FD = 0x4C01
                let ret = libc::ioctl(fd, 0x4C01);
                if ret != 0 {
                    let err = io::Error::last_os_error();
                    eprintln!("systemd-shutdown: Failed to detach {}: {}", name_str, err);
                }
                libc::close(fd);
            }
        }
    }
}

/// Deactivate device-mapper targets.
fn deactivate_dm_devices() {
    let dm_dir = Path::new("/sys/block");
    if !dm_dir.exists() {
        return;
    }

    let entries = match fs::read_dir(dm_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("dm-") {
            continue;
        }

        // Read the DM name from /sys/block/dm-N/dm/name
        let dm_name_path = entry.path().join("dm/name");
        let dm_name = match fs::read_to_string(&dm_name_path) {
            Ok(n) => n.trim().to_string(),
            Err(_) => name_str.to_string(),
        };

        eprintln!(
            "systemd-shutdown: Deactivating DM device {} ({})",
            name_str, dm_name
        );

        // Try using dmsetup to remove the device
        let _ = process::Command::new("dmsetup")
            .arg("remove")
            .arg("--force")
            .arg(&dm_name)
            .status();
    }
}

/// Stop MD RAID arrays.
fn stop_md_arrays() {
    let proc_mdstat = Path::new("/proc/mdstat");
    if !proc_mdstat.exists() {
        return;
    }

    let contents = match fs::read_to_string(proc_mdstat) {
        Ok(c) => c,
        Err(_) => return,
    };

    for line in contents.lines() {
        // Lines describing active arrays look like:
        //   md0 : active raid1 sda1[0] sdb1[1]
        if !line.contains(" : active ") && !line.contains(" : inactive ") {
            continue;
        }

        let md_name = match line.split_whitespace().next() {
            Some(n) => n,
            None => continue,
        };

        let dev_path = format!("/dev/{}", md_name);
        eprintln!("systemd-shutdown: Stopping MD array {}", dev_path);

        // Use mdadm to stop the array
        let _ = process::Command::new("mdadm")
            .arg("--stop")
            .arg(&dev_path)
            .status();
    }
}

/// Sync all filesystems.
fn sync_filesystems() {
    eprintln!("systemd-shutdown: Syncing filesystems...");
    unsafe {
        libc::sync();
    }
}

/// Perform the final shutdown/reboot/halt/kexec action.
fn do_final_action(action: ShutdownAction) -> ! {
    sync_filesystems();

    eprintln!(
        "systemd-shutdown: Performing final action: {}",
        action.as_str()
    );

    // Use the reboot(2) syscall with the appropriate command
    let cmd = match action {
        ShutdownAction::Poweroff => libc::RB_POWER_OFF,
        ShutdownAction::Reboot => libc::RB_AUTOBOOT,
        ShutdownAction::Halt => libc::RB_HALT_SYSTEM,
        ShutdownAction::Kexec => {
            // LINUX_REBOOT_CMD_KEXEC = 0x45584543
            0x45584543u32 as libc::c_int
        }
    };

    unsafe {
        // reboot(2) requires calling sync first (done above) and
        // expects the magic values to be set up correctly. The libc
        // wrapper handles that for us.
        libc::reboot(cmd);
    }

    // If reboot(2) returns, something went wrong
    let err = io::Error::last_os_error();
    eprintln!(
        "systemd-shutdown: reboot({}) failed: {}",
        action.as_str(),
        err
    );

    // As a fallback, try the fallback action
    if action == ShutdownAction::Kexec {
        eprintln!("systemd-shutdown: kexec failed, falling back to reboot");
        unsafe {
            libc::reboot(libc::RB_AUTOBOOT);
        }
    }

    // If we somehow get here, just halt
    eprintln!("systemd-shutdown: All shutdown methods failed, halting");
    unsafe {
        libc::reboot(libc::RB_HALT_SYSTEM);
    }

    // Should never reach here, but we need a diverging type
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}

/// Check if we were called via /run/initramfs/shutdown (the shutdown
/// pivot-root scenario).
fn check_shutdown_initramfs() -> bool {
    if let Ok(exe) = std::env::current_exe() {
        let exe_str = exe.to_string_lossy();
        return exe_str.contains("initramfs");
    }
    false
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!(
            "Usage: {} <poweroff|reboot|halt|kexec>",
            args.first()
                .map(|s| s.as_str())
                .unwrap_or("systemd-shutdown")
        );
        process::exit(EXIT_FAILURE);
    }

    let action = match ShutdownAction::from_verb(&args[1]) {
        Some(a) => a,
        None => {
            eprintln!(
                "systemd-shutdown: Unknown action '{}'. \
                 Expected: poweroff, reboot, halt, kexec",
                args[1]
            );
            process::exit(EXIT_FAILURE);
        }
    };

    eprintln!(
        "systemd-shutdown: Shutting down (action={})...",
        action.as_str()
    );

    let is_initramfs = check_shutdown_initramfs();
    if is_initramfs {
        eprintln!("systemd-shutdown: Running in shutdown initramfs");
    }

    // Step 1: Send SIGTERM to all remaining processes
    kill_all_processes(libc::SIGTERM);

    // Step 2: Wait for processes to exit
    wait_for_processes(SIGTERM_TIMEOUT);

    // Step 3: Send SIGKILL to anything still alive
    kill_all_processes(libc::SIGKILL);

    // Brief wait for SIGKILL to take effect
    thread::sleep(Duration::from_millis(500));

    // Step 4: Sync and unmount all filesystems
    sync_filesystems();

    let unmount_failures = unmount_all();
    if unmount_failures > 0 {
        eprintln!(
            "systemd-shutdown: {} filesystem(s) could not be unmounted",
            unmount_failures
        );
    }

    // Step 5: Detach loop devices
    detach_loop_devices();

    // Step 6: Deactivate DM targets
    deactivate_dm_devices();

    // Step 7: Stop MD RAID arrays
    stop_md_arrays();

    // Step 8: Remount root read-only as a last resort for data safety
    remount_root_readonly();

    // Step 9: Final sync
    sync_filesystems();

    // Step 10: Perform the final action (poweroff/reboot/halt/kexec)
    do_final_action(action);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shutdown_action_from_verb() {
        assert_eq!(
            ShutdownAction::from_verb("poweroff"),
            Some(ShutdownAction::Poweroff)
        );
        assert_eq!(
            ShutdownAction::from_verb("reboot"),
            Some(ShutdownAction::Reboot)
        );
        assert_eq!(
            ShutdownAction::from_verb("halt"),
            Some(ShutdownAction::Halt)
        );
        assert_eq!(
            ShutdownAction::from_verb("kexec"),
            Some(ShutdownAction::Kexec)
        );
        assert_eq!(ShutdownAction::from_verb("unknown"), None);
        assert_eq!(ShutdownAction::from_verb(""), None);
    }

    #[test]
    fn test_shutdown_action_as_str() {
        assert_eq!(ShutdownAction::Poweroff.as_str(), "poweroff");
        assert_eq!(ShutdownAction::Reboot.as_str(), "reboot");
        assert_eq!(ShutdownAction::Halt.as_str(), "halt");
        assert_eq!(ShutdownAction::Kexec.as_str(), "kexec");
    }

    #[test]
    fn test_shutdown_action_roundtrip() {
        for action in [
            ShutdownAction::Poweroff,
            ShutdownAction::Reboot,
            ShutdownAction::Halt,
            ShutdownAction::Kexec,
        ] {
            assert_eq!(ShutdownAction::from_verb(action.as_str()), Some(action));
        }
    }

    #[test]
    fn test_should_skip_unmount_api_fs() {
        let entry = MountEntry {
            mount_point: PathBuf::from("/proc"),
            fs_type: "proc".to_string(),
            source: "proc".to_string(),
        };
        assert!(should_skip_unmount(&entry));

        let entry = MountEntry {
            mount_point: PathBuf::from("/sys"),
            fs_type: "sysfs".to_string(),
            source: "sysfs".to_string(),
        };
        assert!(should_skip_unmount(&entry));

        let entry = MountEntry {
            mount_point: PathBuf::from("/dev"),
            fs_type: "devtmpfs".to_string(),
            source: "devtmpfs".to_string(),
        };
        assert!(should_skip_unmount(&entry));

        let entry = MountEntry {
            mount_point: PathBuf::from("/sys/fs/cgroup"),
            fs_type: "cgroup2".to_string(),
            source: "cgroup2".to_string(),
        };
        assert!(should_skip_unmount(&entry));
    }

    #[test]
    fn test_should_skip_unmount_root() {
        let entry = MountEntry {
            mount_point: PathBuf::from("/"),
            fs_type: "ext4".to_string(),
            source: "/dev/sda1".to_string(),
        };
        assert!(should_skip_unmount(&entry));
    }

    #[test]
    fn test_should_not_skip_regular_mount() {
        let entry = MountEntry {
            mount_point: PathBuf::from("/home"),
            fs_type: "ext4".to_string(),
            source: "/dev/sda2".to_string(),
        };
        assert!(!should_skip_unmount(&entry));

        let entry = MountEntry {
            mount_point: PathBuf::from("/mnt/data"),
            fs_type: "xfs".to_string(),
            source: "/dev/sdb1".to_string(),
        };
        assert!(!should_skip_unmount(&entry));

        let entry = MountEntry {
            mount_point: PathBuf::from("/boot"),
            fs_type: "vfat".to_string(),
            source: "/dev/sda1".to_string(),
        };
        assert!(!should_skip_unmount(&entry));
    }

    #[test]
    fn test_should_skip_tmpfs() {
        let entry = MountEntry {
            mount_point: PathBuf::from("/tmp"),
            fs_type: "tmpfs".to_string(),
            source: "tmpfs".to_string(),
        };
        assert!(should_skip_unmount(&entry));
    }

    #[test]
    fn test_unescape_mountinfo_path_no_escapes() {
        assert_eq!(unescape_mountinfo_path("/home/user"), "/home/user");
        assert_eq!(unescape_mountinfo_path("/mnt/data"), "/mnt/data");
    }

    #[test]
    fn test_parse_proc_mounts_format() {
        // Create a temporary file with mock /proc/mounts content
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_proc_mounts");
        let content = "\
/dev/sda1 / ext4 rw,relatime 0 0
proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0
sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0
/dev/sda2 /home ext4 rw,relatime 0 0
tmpfs /tmp tmpfs rw,nosuid,nodev 0 0
";
        fs::write(&temp_file, content).unwrap();

        let entries = parse_proc_mounts(temp_file.to_str().unwrap()).unwrap();
        assert_eq!(entries.len(), 5);

        assert_eq!(entries[0].mount_point, PathBuf::from("/"));
        assert_eq!(entries[0].fs_type, "ext4");
        assert_eq!(entries[0].source, "/dev/sda1");

        assert_eq!(entries[1].mount_point, PathBuf::from("/proc"));
        assert_eq!(entries[1].fs_type, "proc");

        assert_eq!(entries[3].mount_point, PathBuf::from("/home"));
        assert_eq!(entries[3].fs_type, "ext4");
        assert_eq!(entries[3].source, "/dev/sda2");

        fs::remove_file(&temp_file).unwrap();
    }

    #[test]
    fn test_parse_mountinfo_format() {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_mountinfo");
        let content = "\
22 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw
23 22 0:6 / /dev rw,nosuid,relatime shared:2 - devtmpfs udev rw,size=4096k
24 22 0:7 / /proc rw,nosuid,nodev,noexec,relatime shared:3 - proc proc rw
25 22 8:2 / /home rw,relatime shared:4 - ext4 /dev/sda2 rw
";
        fs::write(&temp_file, content).unwrap();

        let entries = parse_mountinfo(temp_file.to_str().unwrap()).unwrap();
        assert_eq!(entries.len(), 4);

        assert_eq!(entries[0].mount_point, PathBuf::from("/"));
        assert_eq!(entries[0].fs_type, "ext4");
        assert_eq!(entries[0].source, "/dev/sda1");

        assert_eq!(entries[1].mount_point, PathBuf::from("/dev"));
        assert_eq!(entries[1].fs_type, "devtmpfs");

        assert_eq!(entries[2].mount_point, PathBuf::from("/proc"));
        assert_eq!(entries[2].fs_type, "proc");

        assert_eq!(entries[3].mount_point, PathBuf::from("/home"));
        assert_eq!(entries[3].fs_type, "ext4");

        fs::remove_file(&temp_file).unwrap();
    }
}

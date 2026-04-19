//! rust-systemd-stage2 — drop-in replacement for NixOS's bash
//! `stage-2-init.sh` that runs after stage-1 initrd has `switch_root`ed
//! onto the real rootfs.
//!
//! Why this exists: upstream `stage-2-init.sh` contains a bash process
//! substitution that redirects the script's stdout/stderr through
//!
//!     exec > >(tee -i /proc/self/fd/"$logOutFd" | while read -r line; do
//!         echo "<7>stage-2-init: $line" > /dev/kmsg
//!     done) 2>&1
//!
//! This fd-inheritance dance races with parallel kernel module loading
//! (fuse/vmci/vsock auto-load) during early boot and hangs about 30%
//! of the time in the NixOS VM test framework.  The rust equivalent
//! just writes to /dev/console directly — no subshell, no pipe, no
//! race.
//!
//! The binary is installed into `/init` on the rootfs in place of the
//! bash `stage-2-init.sh`.  It runs as PID 1 after stage-1 `switch_root`
//! and:
//!
//!   1. Remounts `/` read-write (stage-1 may have mounted it ro).
//!   2. Ensures `/nix/store` is mounted `ro,nodev,nosuid`.
//!   3. Mounts the essential API VFS (`/proc`, `/sys`, `/dev`, `/run`)
//!      if stage-1 didn't already move them over.
//!   4. Runs `$systemConfig/activate` to install /etc, wrappers, PAM/NSS.
//!   5. Records `/run/booted-system` → `$systemConfig`.
//!   6. Execs `$systemConfig/systemd/lib/systemd/systemd` (rust-systemd)
//!      with argv[0] set to `systemd` so the multi-call dispatch picks
//!      up the service-manager branch.
//!
//! `$systemConfig` is derived from the binary's own path: NixOS
//! symlinks `/init -> <systemConfig>/init`, so `current_exe()`'s parent
//! is the system configuration store path.

use std::ffi::CString;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() -> ! {
    log("rust-systemd-stage2: entered main");

    let system_config = system_config_from_cmdline()
        .unwrap_or_else(|| PathBuf::from("/run/current-system"));

    log(&format!(
        "rust-systemd-stage2: systemConfig={}",
        system_config.display()
    ));

    log("rust-systemd-stage2: remount_root_rw");
    remount_root_rw();
    log("rust-systemd-stage2: ensure_api_mounts");
    ensure_api_mounts();
    log("rust-systemd-stage2: apply_nix_store_mount_options");
    apply_nix_store_mount_options();
    log("rust-systemd-stage2: install_initial_dirs");
    install_initial_dirs();
    log("rust-systemd-stage2: run_activation");
    run_activation(&system_config);
    log("rust-systemd-stage2: mark_booted_system");
    mark_booted_system(&system_config);
    // /run/current-system is the primary symlink used by almost every
    // tool on the system — set it alongside booted-system so the
    // first-boot exec can find systemd via /run/current-system.
    let cur = Path::new("/run/current-system");
    let _ = std::fs::remove_file(cur);
    let _ = std::os::unix::fs::symlink(&system_config, cur);

    log("rust-systemd-stage2: exec_systemd");
    exec_systemd(&system_config);
}

/// Parse /proc/cmdline for `init=<path>` and return the parent dir as
/// the systemConfig store path.  Kernel sets this from the bootloader
/// entry; NixOS populates it as `/nix/store/<hash>-nixos-system-xxx/init`.
fn system_config_from_cmdline() -> Option<PathBuf> {
    let cmdline = std::fs::read_to_string("/proc/cmdline").ok()?;
    for word in cmdline.split_whitespace() {
        if let Some(path) = word.strip_prefix("init=") {
            let init = PathBuf::from(path);
            return init.parent().map(|p| p.to_path_buf());
        }
    }
    None
}

/// Write a line to stderr (inherited from bash wrapper; goes to
/// /dev/console) AND to /dev/kmsg (kernel log buffer) so the message
/// survives even if stderr is redirected.
fn log(msg: &str) {
    let line = format!("{msg}\n");
    eprint!("{line}");
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open("/dev/kmsg") {
        use std::io::Write;
        let _ = f.write_all(format!("<6>{line}").as_bytes());
    }
}

fn remount_root_rw() {
    if let Err(e) = nix::mount::mount::<str, str, str, str>(
        None,
        "/",
        None,
        nix::mount::MsFlags::MS_REMOUNT,
        None,
    ) {
        log(&format!("rust-systemd-stage2: root rw remount failed: {e}"));
    }
}

/// Ensure /proc, /sys, /dev, /run are mounted.  Stage-1 usually moved
/// these into the new root, but if we're invoked directly (soft reboot,
/// container start) they may still need mounting.
fn ensure_api_mounts() {
    let specs: &[(&str, &str, &str, &str)] = &[
        ("proc", "/proc", "proc", ""),
        ("sysfs", "/sys", "sysfs", ""),
        ("devtmpfs", "/dev", "devtmpfs", "mode=0755"),
        ("tmpfs", "/run", "tmpfs", "mode=0755,size=25%"),
    ];
    for &(fstype, mountpoint, source, data) in specs {
        if is_mounted(mountpoint) {
            continue;
        }
        let _ = std::fs::create_dir_all(mountpoint);
        let data_opt: Option<&str> = if data.is_empty() { None } else { Some(data) };
        if let Err(e) = nix::mount::mount(
            Some(source),
            mountpoint,
            Some(fstype),
            nix::mount::MsFlags::empty(),
            data_opt,
        ) {
            log(&format!(
                "rust-systemd-stage2: mount {fstype} on {mountpoint} failed: {e}"
            ));
        }
    }
}

fn is_mounted(path: &str) -> bool {
    let Ok(content) = std::fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    for line in content.lines() {
        // mountinfo field 5 is the mount point
        if let Some(mp) = line.split_whitespace().nth(4)
            && mp == path
        {
            return true;
        }
    }
    false
}

/// Re-mount /nix/store with ro,nodev,nosuid if any of those options
/// are missing.  Matches the upstream stage-2 logic.
fn apply_nix_store_mount_options() {
    let Ok(content) = std::fs::read_to_string("/proc/self/mountinfo") else {
        return;
    };
    let mut current_opts: Option<String> = None;
    for line in content.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 6 && fields[4] == "/nix/store" {
            // Per-mount options are field 6 (0-indexed 5)
            current_opts = Some(fields[5].to_string());
        }
    }
    let Some(opts) = current_opts else { return };
    let wanted = ["ro", "nodev", "nosuid"];
    let missing: Vec<&&str> = wanted
        .iter()
        .filter(|w| !opts.split(',').any(|o| &o == *w))
        .collect();
    if missing.is_empty() {
        return;
    }

    // Bind-mount over itself and then remount with the missing options.
    if let Err(e) = nix::mount::mount::<str, str, str, str>(
        Some("/nix/store"),
        "/nix/store",
        None,
        nix::mount::MsFlags::MS_BIND,
        None,
    ) {
        log(&format!(
            "rust-systemd-stage2: /nix/store bind-mount failed: {e}"
        ));
        return;
    }
    let missing_str = missing
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let mut flags = nix::mount::MsFlags::MS_REMOUNT | nix::mount::MsFlags::MS_BIND;
    for w in &missing {
        match **w {
            "ro" => flags |= nix::mount::MsFlags::MS_RDONLY,
            "nodev" => flags |= nix::mount::MsFlags::MS_NODEV,
            "nosuid" => flags |= nix::mount::MsFlags::MS_NOSUID,
            _ => {}
        }
    }
    if let Err(e) = nix::mount::mount::<str, str, str, str>(
        None,
        "/nix/store",
        None,
        flags,
        None,
    ) {
        log(&format!(
            "rust-systemd-stage2: /nix/store remount {missing_str} failed: {e}"
        ));
    }
}

/// Create directories the activation script expects to exist.
fn install_initial_dirs() {
    let _ = std::fs::create_dir_all("/etc");
    chmod_path("/etc", 0o755);
    if !Path::new("/etc/nixos").is_symlink() {
        let _ = std::fs::create_dir_all("/etc/nixos");
    }
    let _ = std::fs::create_dir_all("/tmp");
    chmod_path("/tmp", 0o1777);
}

/// Thin wrapper over libc::chmod — the nix crate's fchmodat API
/// changed between versions in ways that made the call site verbose.
fn chmod_path(path: &str, mode: u32) {
    let Ok(cpath) = CString::new(path) else {
        return;
    };
    unsafe {
        libc::chmod(cpath.as_ptr(), mode as libc::mode_t);
    }
}

/// Run `$systemConfig/activate`, streaming its output to /dev/console.
/// A non-zero exit is logged but not fatal — real systemd behaves the
/// same way (activation snippets are allowed to fail individually).
fn run_activation(system_config: &Path) {
    let activate = system_config.join("activate");
    if !activate.exists() {
        log(&format!(
            "rust-systemd-stage2: no activate script at {}",
            activate.display()
        ));
        return;
    }
    log(&format!(
        "rust-systemd-stage2: running activation script {}",
        activate.display()
    ));

    // Open /dev/console once and use it as both stdout and stderr for
    // the child — avoids the tee-pipe setup that bash stage-2 used.
    let console_fd = unsafe {
        libc::open(
            c"/dev/console".as_ptr(),
            libc::O_WRONLY | libc::O_NOCTTY | libc::O_CLOEXEC,
        )
    };
    let mut cmd = Command::new(&activate);
    if console_fd >= 0 {
        let dup_out = unsafe { libc::dup(console_fd) };
        let dup_err = unsafe { libc::dup(console_fd) };
        if dup_out >= 0 {
            use std::os::fd::FromRawFd;
            cmd.stdout(unsafe { std::process::Stdio::from_raw_fd(dup_out) });
        }
        if dup_err >= 0 {
            use std::os::fd::FromRawFd;
            cmd.stderr(unsafe { std::process::Stdio::from_raw_fd(dup_err) });
        }
        unsafe { libc::close(console_fd) };
    }
    match cmd.status() {
        Ok(s) => log(&format!(
            "rust-systemd-stage2: activate exit={}",
            s.code().unwrap_or(-1)
        )),
        Err(e) => log(&format!("rust-systemd-stage2: activate spawn failed: {e}")),
    }
}

fn mark_booted_system(system_config: &Path) {
    let link = Path::new("/run/booted-system");
    let _ = std::fs::remove_file(link);
    if let Err(e) = std::os::unix::fs::symlink(system_config, link) {
        log(&format!(
            "rust-systemd-stage2: symlink /run/booted-system failed: {e}"
        ));
    }
}

/// Final step: `execv` the real systemd binary.  PID 1 stays the same
/// process, just with new code — no fork, no wait.
fn exec_systemd(system_config: &Path) -> ! {
    let candidates = [
        system_config.join("systemd/lib/systemd/systemd"),
        PathBuf::from("/run/current-system/systemd/lib/systemd/systemd"),
    ];
    let systemd = candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .unwrap_or_else(|| candidates[0].clone());

    log(&format!("rust-systemd-stage2: exec {}", systemd.display()));

    let path_c = CString::new(systemd.as_os_str().as_encoded_bytes())
        .expect("stage2: systemd path has NUL byte");
    // argv[0] = "systemd" so the service-manager branch fires.
    let argv0 = CString::new("systemd").unwrap();
    let args: Vec<&CString> = vec![&argv0];
    let argv_ptrs: Vec<*const libc::c_char> = args
        .iter()
        .map(|s| s.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    // Inherit the current environment — NixOS stage-1 already stripped
    // it with `env -i`, so this is effectively empty which matches what
    // bash stage-2 would hand off.
    unsafe {
        libc::execv(path_c.as_ptr(), argv_ptrs.as_ptr());
    }
    let err = std::io::Error::last_os_error();
    log(&format!("rust-systemd-stage2: execv failed: {err}"));
    // Fall back to using std::process::Command (which goes through the
    // executor helper path) so PID 1 doesn't just exit silently.
    let _ = Command::new(&systemd).arg0("systemd").exec();
    // If that also fails, kernel will panic since PID 1 returned.
    std::process::exit(1);
}

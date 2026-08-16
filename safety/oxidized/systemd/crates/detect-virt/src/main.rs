//! systemd-detect-virt — Detect execution in a virtualized environment.
//!
//! A drop-in replacement for `systemd-detect-virt(1)`. Detects whether
//! the system is running inside a virtual machine, a container, or a
//! chroot, and prints the identifier of the virtualization technology.
//!
//! Exit codes:
//!   0 — Virtualization was detected (or chroot when --chroot)
//!   1 — No virtualization detected (or not in a chroot when --chroot)
//!
//! Supported options:
//!
//! - `--vm` / `-v`         — Only detect hardware virtualization (VMs)
//! - `--container` / `-c`  — Only detect container virtualization
//! - `--chroot` / `-r`     — Detect whether running in a chroot
//! - `--private-users`     — Detect whether running in a user namespace
//! - `--quiet` / `-q`      — Suppress output, only set exit code
//! - `--list`              — List known virtualization technologies

use clap::Parser;
use std::fs;
use std::path::Path;
use std::process;

#[derive(Parser, Debug)]
#[command(
    name = "systemd-detect-virt",
    about = "Detect execution in a virtualized environment",
    version
)]
struct Cli {
    /// Only detect hardware virtualization (VMs).
    #[arg(short = 'v', long)]
    vm: bool,

    /// Only detect container-based virtualization.
    #[arg(short = 'c', long)]
    container: bool,

    /// Detect whether running in a chroot environment.
    #[arg(short = 'r', long)]
    chroot: bool,

    /// Detect whether running in a user namespace.
    #[arg(long)]
    private_users: bool,

    /// Suppress output; only set the exit code.
    #[arg(short, long)]
    quiet: bool,

    /// List known virtualization technologies and exit.
    #[arg(long)]
    list: bool,
}

/// Known VM identifiers, ordered to match C's `Virtualization` enum so `--list`
/// prints them in the same sequence as `systemd-detect-virt --list`. These feed
/// only `--list`; detection returns its identifiers directly. `cloud-hypervisor`
/// and `firecracker` are rust-only forward-ports (systemd 260's enum has
/// neither, using a single `vm-other` fallback that rust's detection never
/// emits), so they are appended where `vm-other` sits in C.
const KNOWN_VMS: &[&str] = &[
    "kvm",
    "amazon",
    "qemu",
    "bochs",
    "xen",
    "uml",
    "vmware",
    "oracle",
    "microsoft",
    "zvm",
    "parallels",
    "bhyve",
    "qnx",
    "acrn",
    "powervm",
    "apple",
    "sre",
    "google",
    "cloud-hypervisor",
    "firecracker",
];

/// Known container identifiers, ordered to match C's `Virtualization` enum.
const KNOWN_CONTAINERS: &[&str] = &[
    "systemd-nspawn",
    "lxc-libvirt",
    "lxc",
    "openvz",
    "docker",
    "podman",
    "rkt",
    "wsl",
    "proot",
    "pouch",
    "container-other",
];

/// The full sequence printed by `--list`: `none`, then VMs, then containers, in
/// C enum order. Kept as a function so the exact sequence can be unit-tested.
fn known_technologies() -> Vec<&'static str> {
    let mut list = Vec::with_capacity(1 + KNOWN_VMS.len() + KNOWN_CONTAINERS.len());
    list.push("none");
    list.extend_from_slice(KNOWN_VMS);
    list.extend_from_slice(KNOWN_CONTAINERS);
    list
}

/// Try to read a file to a string, trimming trailing whitespace.
fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Detect VM via DMI/SMBIOS product name and sys_vendor.
fn detect_vm_dmi() -> Option<&'static str> {
    let product = read_trimmed("/sys/class/dmi/id/product_name").unwrap_or_default();
    let vendor = read_trimmed("/sys/class/dmi/id/sys_vendor").unwrap_or_default();
    let board = read_trimmed("/sys/class/dmi/id/board_vendor").unwrap_or_default();
    let bios = read_trimmed("/sys/class/dmi/id/bios_vendor").unwrap_or_default();
    let product_lower = product.to_lowercase();
    let vendor_lower = vendor.to_lowercase();
    let board_lower = board.to_lowercase();
    let bios_lower = bios.to_lowercase();

    let combined = format!(
        "{} {} {} {}",
        product_lower, vendor_lower, board_lower, bios_lower
    );

    if combined.contains("qemu") || combined.contains("kvm") {
        // Distinguish QEMU+KVM from plain QEMU
        if combined.contains("kvm") {
            return Some("kvm");
        }
        return Some("qemu");
    }
    if combined.contains("vmware") || combined.contains("vmw") {
        return Some("vmware");
    }
    if combined.contains("microsoft") && combined.contains("virtual") {
        return Some("microsoft");
    }
    if combined.contains("innotek")
        || combined.contains("virtualbox")
        || combined.contains("oracle") && combined.contains("virtual")
    {
        return Some("oracle");
    }
    if combined.contains("xen") {
        return Some("xen");
    }
    if combined.contains("parallels") {
        return Some("parallels");
    }
    if combined.contains("bochs") {
        return Some("bochs");
    }
    if combined.contains("bhyve") {
        return Some("bhyve");
    }
    if combined.contains("amazon ec2")
        || combined.contains("amazon") && (combined.contains("nitro") || combined.contains("ec2"))
    {
        return Some("amazon");
    }
    if combined.contains("google") {
        return Some("google");
    }
    if combined.contains("cloud hypervisor") || combined.contains("cloud-hypervisor") {
        return Some("cloud-hypervisor");
    }
    if combined.contains("firecracker") {
        return Some("firecracker");
    }
    if combined.contains("apple virtualization") {
        return Some("apple");
    }
    if combined.contains("acrn") {
        return Some("acrn");
    }
    if combined.contains("powervm") {
        return Some("powervm");
    }

    None
}

/// Detect VM via CPUID hypervisor bit and hypervisor vendor string.
/// Only works on x86/x86_64.
fn detect_vm_cpuid() -> Option<&'static str> {
    // Read from /proc/cpuinfo for hypervisor flag and model name
    let cpuinfo = read_trimmed("/proc/cpuinfo").unwrap_or_default();

    // Check for the "hypervisor" flag (CPUID bit 31 of ECX for leaf 1)
    let has_hypervisor_flag = cpuinfo.lines().any(|line| {
        if let Some(flags) = line.strip_prefix("flags") {
            flags.split_whitespace().any(|f| f == "hypervisor")
        } else {
            false
        }
    });

    if !has_hypervisor_flag {
        return None;
    }

    // Try to read the hypervisor_vendor from /sys/hypervisor/type
    if let Some(hv_type) = read_trimmed("/sys/hypervisor/type")
        && hv_type.as_str() == "xen"
    {
        return Some("xen");
    }

    // Check model name for hints
    let model_lower = cpuinfo.to_lowercase();
    if model_lower.contains("qemu") {
        return Some("qemu");
    }
    if model_lower.contains("kvm") {
        return Some("kvm");
    }

    // We know there's a hypervisor but can't identify it further from cpuinfo.
    // Fall through to let DMI or device-tree detection identify it.
    None
}

/// Detect VM via device tree (common on ARM/aarch64).
fn detect_vm_device_tree() -> Option<&'static str> {
    // /proc/device-tree/compatible or /sys/firmware/devicetree/base/compatible
    let compatible_path = "/proc/device-tree/compatible";
    if let Some(compat) = read_trimmed(compatible_path) {
        let compat_lower = compat.to_lowercase();
        if compat_lower.contains("qemu") {
            return Some("qemu");
        }
        if compat_lower.contains("kvm") {
            return Some("kvm");
        }
        if compat_lower.contains("xen") {
            return Some("xen");
        }
        if compat_lower.contains("vmware") {
            return Some("vmware");
        }
    }

    // Check hypervisor node
    let hypervisor_compat = "/proc/device-tree/hypervisor/compatible";
    if let Some(compat) = read_trimmed(hypervisor_compat) {
        let compat_lower = compat.to_lowercase();
        if compat_lower.contains("kvm") || compat_lower.contains("qemu") {
            return Some("kvm");
        }
        if compat_lower.contains("xen") {
            return Some("xen");
        }
    }

    None
}

/// Detect User-Mode Linux.
fn detect_vm_uml() -> Option<&'static str> {
    if let Some(cpuinfo) = read_trimmed("/proc/cpuinfo")
        && cpuinfo.contains("User Mode Linux")
    {
        return Some("uml");
    }
    None
}

/// Detect VM via /sys/hypervisor/type.
fn detect_vm_hypervisor() -> Option<&'static str> {
    if let Some(hv_type) = read_trimmed("/sys/hypervisor/type")
        && hv_type.as_str() == "xen"
    {
        return Some("xen");
    }
    None
}

/// Main VM detection entry point: tries multiple methods.
fn detect_vm() -> Option<&'static str> {
    // UML is special — it shows up in /proc/cpuinfo
    if let Some(v) = detect_vm_uml() {
        return Some(v);
    }

    // DMI/SMBIOS is the most reliable on x86
    if let Some(v) = detect_vm_dmi() {
        return Some(v);
    }

    // CPUID hypervisor flag
    if let Some(v) = detect_vm_cpuid() {
        return Some(v);
    }

    // Device tree (ARM)
    if let Some(v) = detect_vm_device_tree() {
        return Some(v);
    }

    // /sys/hypervisor
    if let Some(v) = detect_vm_hypervisor() {
        return Some(v);
    }

    None
}

/// Detect container via various indicators.
fn detect_container() -> Option<&'static str> {
    // 1. Check /run/systemd/container (set by systemd-nspawn and others)
    if let Some(container) = read_trimmed("/run/systemd/container") {
        return match container.as_str() {
            "systemd-nspawn" => Some("systemd-nspawn"),
            "docker" => Some("docker"),
            "podman" => Some("podman"),
            "lxc" => Some("lxc"),
            "lxc-libvirt" => Some("lxc-libvirt"),
            "rkt" => Some("rkt"),
            "wsl" => Some("wsl"),
            "proot" => Some("proot"),
            "pouch" => Some("pouch"),
            _ => Some("container-other"),
        };
    }

    // 2. Check the container= environment variable of PID 1
    if let Ok(environ) = fs::read("/proc/1/environ") {
        // environ is NUL-separated
        for entry in environ.split(|&b| b == 0) {
            if let Ok(s) = std::str::from_utf8(entry)
                && let Some(val) = s.strip_prefix("container=")
            {
                return match val {
                    "systemd-nspawn" => Some("systemd-nspawn"),
                    "docker" => Some("docker"),
                    "podman" => Some("podman"),
                    "lxc" => Some("lxc"),
                    "lxc-libvirt" => Some("lxc-libvirt"),
                    "rkt" => Some("rkt"),
                    "wsl" | "wsl2" => Some("wsl"),
                    "proot" => Some("proot"),
                    "pouch" => Some("pouch"),
                    _ => Some("container-other"),
                };
            }
        }
    }

    // 3. Check for /.dockerenv or /.dockerinit
    if Path::new("/.dockerenv").exists() || Path::new("/.dockerinit").exists() {
        return Some("docker");
    }

    // 4. Check /proc/1/cgroup for container indicators
    if let Some(cgroup) = read_trimmed("/proc/1/cgroup") {
        let cgroup_lower = cgroup.to_lowercase();
        if cgroup_lower.contains("/docker/") || cgroup_lower.contains("/docker-") {
            return Some("docker");
        }
        if cgroup_lower.contains("/lxc/") || cgroup_lower.contains("/lxc.") {
            return Some("lxc");
        }
        if cgroup_lower.contains("/libpod-") || cgroup_lower.contains("/podman-") {
            return Some("podman");
        }
    }

    // 5. Check for OpenVZ
    if Path::new("/proc/vz").exists() && !Path::new("/proc/bc").exists() {
        return Some("openvz");
    }

    // 6. Check for WSL
    if let Some(version) = read_trimmed("/proc/version") {
        let version_lower = version.to_lowercase();
        if version_lower.contains("microsoft") || version_lower.contains("wsl") {
            return Some("wsl");
        }
    }

    // 7. Check /proc/1/sched — in some containers PID 1 in /proc/1/sched
    //    shows a different PID
    if let Some(sched) = read_trimmed("/proc/1/sched")
        && let Some(first_line) = sched.lines().next()
    {
        // In containers the first line is often "bash (PID)" where PID != 1
        // But this is unreliable, so we only use it as a last resort
        if first_line.contains("(") {
            // Parse the PID from the first line
            if let Some(start) = first_line.rfind('(') {
                let rest = &first_line[start + 1..];
                if let Some(end) = rest.find(')')
                    && let Ok(pid) = rest[..end].trim().parse::<u64>()
                    && pid != 1
                {
                    return Some("container-other");
                }
            }
        }
    }

    // 8. The root PID namespace has a hardcoded inode number of 0xEFFFFFFC
    //    (PROC_PID_INIT_INO, stable since kernel 3.8).  If our PID namespace
    //    inode differs, we are in a non-root PID namespace, which systemd
    //    reports as an unknown container manager — this is detect_container()'s
    //    final fallback (running_in_pidns()) and is what makes
    //    `unshare --pid systemd-detect-virt --container` succeed.
    {
        use std::os::unix::fs::MetadataExt;
        const PROC_PID_INIT_INO: u64 = 0xEFFF_FFFC;
        if let Ok(meta) = fs::metadata("/proc/self/ns/pid")
            && meta.ino() != PROC_PID_INIT_INO
        {
            return Some("container-other");
        }
    }

    None
}

/// Detect if running in a chroot.
///
/// Compares the device and inode of / with the root of PID 1.
/// If they differ, we're in a chroot.
fn detect_chroot() -> bool {
    // Method 1: Compare / of our process with / of PID 1
    // If /proc/1/root is accessible (requires privileges), compare its
    // device+inode with our /.
    use std::os::unix::fs::MetadataExt;

    let our_root = match fs::metadata("/") {
        Ok(m) => (m.dev(), m.ino()),
        Err(_) => return false,
    };

    // Try /proc/1/root
    if let Ok(pid1_root) = fs::metadata("/proc/1/root/.") {
        let pid1 = (pid1_root.dev(), pid1_root.ino());
        if our_root != pid1 {
            return true;
        }
    }

    false
}

/// Detect if running in a user namespace.
///
/// Checks /proc/self/uid_map — if the mapping is not the identity
/// mapping "0 0 4294967295", we're in a user namespace.
fn detect_private_users() -> bool {
    if let Some(uid_map) = read_trimmed("/proc/self/uid_map") {
        let parts: Vec<&str> = uid_map.split_whitespace().collect();
        if parts.len() >= 3 {
            // Identity mapping: "0 0 4294967295" (or the 32-bit max)
            if parts[0] == "0"
                && parts[1] == "0"
                && let Ok(count) = parts[2].parse::<u64>()
                && count >= 4_294_967_295
            {
                return false; // Full identity mapping, not in a user ns
            }
            return true;
        }
    }
    false
}

fn main() {
    let cli = Cli::parse();

    // --list: enumerate known technologies, one identifier per line, matching C
    // `systemd-detect-virt --list`, which dumps the Virtualization string table
    // in enum order via puts (no header, grouping, or descriptions).
    if cli.list {
        for id in known_technologies() {
            println!("{id}");
        }
        process::exit(0);
    }

    // --chroot
    if cli.chroot {
        let in_chroot = match std::env::var("SYSTEMD_IN_CHROOT") {
            Ok(val) => val == "1",
            Err(_) => detect_chroot(),
        };
        if !cli.quiet {
            if in_chroot {
                println!("yes");
            } else {
                println!("no");
            }
        }
        process::exit(if in_chroot { 0 } else { 1 });
    }

    // --private-users
    if cli.private_users {
        let in_userns = detect_private_users();
        if !cli.quiet {
            if in_userns {
                println!("yes");
            } else {
                println!("no");
            }
        }
        process::exit(if in_userns { 0 } else { 1 });
    }

    // Default: detect VM and/or container
    let detect_vm_flag = cli.vm || !cli.container;
    let detect_container_flag = cli.container || !cli.vm;

    let mut result: Option<&str> = None;

    if detect_vm_flag {
        result = detect_vm();
    }

    if result.is_none() && detect_container_flag {
        result = detect_container();
    }

    match result {
        Some(virt) => {
            if !cli.quiet {
                println!("{virt}");
            }
            process::exit(0);
        }
        None => {
            if !cli.quiet {
                println!("none");
            }
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_vm_does_not_panic() {
        // Should return Some or None without panicking
        let _ = detect_vm();
    }

    #[test]
    fn test_list_matches_c_format_and_order() {
        // C `systemd-detect-virt --list` dumps the Virtualization string table
        // via puts: plain identifiers, one per line, `none` first, VMs then
        // containers in enum order. Verify our list has that shape and order.
        let list = known_technologies();
        assert_eq!(list[0], "none", "none must be listed first, as in C");

        let pos = |s: &str| list.iter().position(|&x| x == s).unwrap();
        // Shared entries follow C's enum order (a representative sample).
        assert!(pos("kvm") < pos("amazon"));
        assert!(pos("amazon") < pos("qemu"));
        assert!(pos("qemu") < pos("bochs"));
        assert!(pos("google") < pos("systemd-nspawn"), "VMs precede containers");
        assert!(pos("systemd-nspawn") < pos("lxc-libvirt"));
        assert!(pos("lxc-libvirt") < pos("lxc"));
        assert!(pos("lxc") < pos("openvz"));
        assert!(pos("openvz") < pos("docker"));
        assert!(pos("docker") < pos("podman"));
        assert!(pos("pouch") < pos("container-other"));

        // No decoration leaks into the identifiers (the old grouped table had
        // "  VMs:" headers and "name  description" columns).
        for id in &list {
            assert!(!id.is_empty(), "identifier must be non-empty");
            assert!(!id.contains(' '), "identifier must be plain: {id:?}");
            assert!(!id.contains(':'), "identifier must be plain: {id:?}");
        }

        // Forward-port identifiers rust detects (C 260's enum lacks these and
        // uses a single vm-other fallback rust does not emit).
        assert!(list.contains(&"cloud-hypervisor"));
        assert!(list.contains(&"firecracker"));
    }

    #[test]
    fn test_detect_container_does_not_panic() {
        let _ = detect_container();
    }

    #[test]
    fn test_detect_chroot_does_not_panic() {
        let _ = detect_chroot();
    }

    #[test]
    fn test_detect_private_users_does_not_panic() {
        let _ = detect_private_users();
    }

    #[test]
    fn test_read_trimmed_nonexistent() {
        assert!(read_trimmed("/nonexistent/path/that/should/not/exist").is_none());
    }

    #[test]
    fn test_read_trimmed_proc() {
        // /proc/self/comm should exist on Linux
        if cfg!(target_os = "linux") {
            let result = read_trimmed("/proc/self/comm");
            assert!(result.is_some());
        }
    }
}

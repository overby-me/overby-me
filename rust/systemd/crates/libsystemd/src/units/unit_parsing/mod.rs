mod device_unit;
mod mount_unit;
mod path_unit;
mod service_unit;
mod slice_unit;
mod socket_unit;
mod swap_unit;
mod target_unit;
mod timer_unit;
mod unit_parser;

pub use device_unit::*;
pub use mount_unit::*;
pub use path_unit::*;
pub use service_unit::*;
pub use slice_unit::*;
pub use socket_unit::*;
pub use swap_unit::*;
pub use target_unit::*;
pub use timer_unit::*;
pub use unit_parser::*;

use log::trace;
use std::path::PathBuf;

pub struct ParsedCommonConfig {
    pub unit: ParsedUnitSection,
    pub install: ParsedInstallSection,
    pub name: String,
    /// The path to the unit file on disk (FragmentPath in systemd terminology).
    pub fragment_path: Option<PathBuf>,
}
pub struct ParsedServiceConfig {
    pub common: ParsedCommonConfig,
    pub srvc: ParsedServiceSection,
}
pub struct ParsedSocketConfig {
    pub common: ParsedCommonConfig,
    pub sock: ParsedSocketSection,
}
pub struct ParsedTargetConfig {
    pub common: ParsedCommonConfig,
}
pub struct ParsedSliceConfig {
    pub common: ParsedCommonConfig,
    pub slice: ParsedSliceSection,
}

pub struct ParsedTimerConfig {
    pub common: ParsedCommonConfig,
    pub timer: ParsedTimerSection,
}

pub struct ParsedPathConfig {
    pub common: ParsedCommonConfig,
    pub path: ParsedPathSection,
}

pub struct ParsedSwapConfig {
    pub common: ParsedCommonConfig,
    pub swap: ParsedSwapSection,
}

/// Parsed configuration for a `.device` unit file.
///
/// Device units in systemd are typically auto-generated from udev events rather
/// than loaded from unit files. However, they support `[Unit]` and `[Install]`
/// sections for dependency management. The `[Device]` section has no directives
/// of its own — all device-specific metadata (SysFSPath, etc.) comes from udev
/// properties at runtime.
pub struct ParsedDeviceConfig {
    pub common: ParsedCommonConfig,
    /// The sysfs path, if known (set at runtime from udev, not from unit files).
    pub sysfs_path: Option<String>,
}

/// Parsed [Slice] section from a `.slice` unit file.
///
/// Slices accept the same resource-control directives as the [Service] section
/// (MemoryMax=, CPUQuota=, TasksMax=, etc.). These are the slice-specific
/// directives; everything else comes from the [Unit] / [Install] sections.
#[derive(Clone, Debug, Default)]
pub struct ParsedSliceSection {
    /// MemoryMin= — minimum memory guarantee. See systemd.resource-control(5).
    pub memory_min: Option<MemoryLimit>,
    /// MemoryLow= — low memory boundary. See systemd.resource-control(5).
    pub memory_low: Option<MemoryLimit>,
    /// MemoryHigh= — throttling memory boundary. See systemd.resource-control(5).
    pub memory_high: Option<MemoryLimit>,
    /// MemoryMax= — hard memory limit. See systemd.resource-control(5).
    pub memory_max: Option<MemoryLimit>,
    /// MemorySwapMax= — hard swap limit. See systemd.resource-control(5).
    pub memory_swap_max: Option<MemoryLimit>,
    /// CPUWeight= — CPU scheduling weight (1–10000). See systemd.resource-control(5).
    pub cpu_weight: Option<CpuWeight>,
    /// StartupCPUWeight= — CPU weight during startup. See systemd.resource-control(5).
    pub startup_cpu_weight: Option<CpuWeight>,
    /// CPUQuota= — CPU time quota as percentage. See systemd.resource-control(5).
    pub cpu_quota: Option<CpuQuota>,
    /// IOWeight= — I/O scheduling weight (1–10000). See systemd.resource-control(5).
    pub io_weight: Option<IoWeight>,
    /// StartupIOWeight= — I/O weight during startup. See systemd.resource-control(5).
    pub startup_io_weight: Option<IoWeight>,
    /// IODeviceWeight= — per-device I/O weight. See systemd.resource-control(5).
    pub io_device_weight: Vec<IoDeviceLimit>,
    /// IOReadBandwidthMax= — per-device read bandwidth limit. See systemd.resource-control(5).
    pub io_read_bandwidth_max: Vec<IoDeviceLimit>,
    /// IOWriteBandwidthMax= — per-device write bandwidth limit. See systemd.resource-control(5).
    pub io_write_bandwidth_max: Vec<IoDeviceLimit>,
    /// IOReadIOPSMax= — per-device read IOPS limit. See systemd.resource-control(5).
    pub io_read_iops_max: Vec<IoDeviceLimit>,
    /// IOWriteIOPSMax= — per-device write IOPS limit. See systemd.resource-control(5).
    pub io_write_iops_max: Vec<IoDeviceLimit>,
    /// TasksMax= — maximum number of tasks (processes + threads). See systemd.resource-control(5).
    pub tasks_max: Option<TasksMax>,
    /// Delegate= — delegate cgroup subtree to the unit. See systemd.resource-control(5).
    pub delegate: Delegate,
    /// CPUAccounting= — enable CPU accounting. See systemd.resource-control(5).
    pub cpu_accounting: Option<bool>,
    /// MemoryAccounting= — enable memory accounting. See systemd.resource-control(5).
    pub memory_accounting: Option<bool>,
    /// IOAccounting= — enable I/O accounting. See systemd.resource-control(5).
    pub io_accounting: Option<bool>,
    /// TasksAccounting= — enable task counting. See systemd.resource-control(5).
    pub tasks_accounting: Option<bool>,
    /// DeviceAllow= — per-device access control entries. See systemd.resource-control(5).
    pub device_allow: Vec<String>,
    /// DevicePolicy= — device access policy. See systemd.resource-control(5).
    pub device_policy: DevicePolicy,
    /// IPAddressAllow= — allowed IP address prefixes. See systemd.resource-control(5).
    pub ip_address_allow: Vec<String>,
    /// IPAddressDeny= — denied IP address prefixes. See systemd.resource-control(5).
    pub ip_address_deny: Vec<String>,
    /// ManagedOOMSwap= — managed OOM swap policy. See systemd.resource-control(5).
    pub managed_oom_swap: Option<String>,
    /// ManagedOOMMemoryPressure= — managed OOM memory pressure policy. See systemd.resource-control(5).
    pub managed_oom_memory_pressure: Option<String>,
    /// ManagedOOMMemoryPressureLimit= — managed OOM pressure threshold. See systemd.resource-control(5).
    pub managed_oom_memory_pressure_limit: Option<String>,
    /// ManagedOOMPreference= — managed OOM preference. See systemd.resource-control(5).
    pub managed_oom_preference: Option<String>,
    /// MemoryPressureWatch= — PSI memory pressure monitoring. See systemd.resource-control(5).
    pub memory_pressure_watch: MemoryPressureWatch,

    // --- New resource-control directives ---
    /// CPUQuotaPeriodSec= — the period for CPU quota enforcement.
    /// See systemd.resource-control(5).
    pub cpu_quota_period_sec: Option<Timeout>,
    /// AllowedCPUs= — CPU set restriction (cpuset controller).
    /// See systemd.resource-control(5).
    pub allowed_cpus: Option<String>,
    /// StartupAllowedCPUs= — CPU set restriction during startup.
    /// See systemd.resource-control(5).
    pub startup_allowed_cpus: Option<String>,
    /// AllowedMemoryNodes= — NUMA node restriction (cpuset controller).
    /// See systemd.resource-control(5).
    pub allowed_memory_nodes: Option<String>,
    /// StartupAllowedMemoryNodes= — NUMA node restriction during startup.
    /// See systemd.resource-control(5).
    pub startup_allowed_memory_nodes: Option<String>,
    /// DefaultMemoryMin= — default memory.min for children.
    /// See systemd.resource-control(5).
    pub default_memory_min: Option<MemoryLimit>,
    /// DefaultMemoryLow= — default memory.low for children.
    /// See systemd.resource-control(5).
    pub default_memory_low: Option<MemoryLimit>,
    /// MemoryZSwapMax= — hard zswap limit. See systemd.resource-control(5).
    pub memory_zswap_max: Option<MemoryLimit>,
    /// IODeviceLatencyTargetSec= — per-device I/O latency target.
    /// See systemd.resource-control(5).
    pub io_device_latency_target_sec: Vec<String>,
    /// DisableControllers= — controllers to disable.
    /// See systemd.resource-control(5).
    pub disable_controllers: Vec<String>,
    /// MemoryPressureThresholdSec= — memory pressure threshold duration.
    /// See systemd.resource-control(5).
    pub memory_pressure_threshold_sec: Option<Timeout>,
    /// IPIngressFilterPath= — BPF ingress filter paths.
    /// See systemd.resource-control(5).
    pub ip_ingress_filter_path: Vec<String>,
    /// IPEgressFilterPath= — BPF egress filter paths.
    /// See systemd.resource-control(5).
    pub ip_egress_filter_path: Vec<String>,
    /// BPFProgram= — BPF programs to attach. See systemd.resource-control(5).
    pub bpf_program: Vec<String>,
    /// SocketBindAllow= — socket bind allow rules.
    /// See systemd.resource-control(5).
    pub socket_bind_allow: Vec<String>,
    /// SocketBindDeny= — socket bind deny rules.
    /// See systemd.resource-control(5).
    pub socket_bind_deny: Vec<String>,
    /// RestrictNetworkInterfaces= — network interface restrictions.
    /// See systemd.resource-control(5).
    pub restrict_network_interfaces: Vec<String>,
    /// NFTSet= — nftables set attachments. See systemd.resource-control(5).
    pub nft_set: Vec<String>,
    /// DelegateSubgroup= — subgroup for delegated cgroup.
    /// See systemd.resource-control(5).
    pub delegate_subgroup: Option<String>,
}

/// Parsed [Path] section from a `.path` unit file.
#[derive(Clone, Debug, Default)]
pub struct ParsedPathSection {
    /// Path watch conditions as (kind, path) pairs.
    /// Kind is one of: "PathExists", "PathExistsGlob", "PathChanged", "PathModified", "DirectoryNotEmpty".
    pub path_exists: Vec<(String, String)>,
    /// MakeDirectory= — create the watched directory before watching (default false).
    pub make_directory: bool,
    /// DirectoryMode= — permission mode for MakeDirectory (default 0o755).
    pub directory_mode: u32,
    /// TriggerLimitIntervalSec= — rate limit interval for path triggers.
    pub trigger_limit_interval_sec: Option<String>,
    /// TriggerLimitBurst= — rate limit burst for path triggers.
    pub trigger_limit_burst: Option<u32>,
    /// Unit= — the unit to activate when the path condition is met (defaults to same-name .service).
    pub unit: Option<String>,
}

/// Parsed [Timer] section from a `.timer` unit file.
#[derive(Clone, Debug, Default)]
pub struct ParsedTimerSection {
    /// OnActiveSec= — relative to the time the timer unit itself is activated.
    pub on_active_sec: Vec<String>,
    /// OnBootSec= — relative to when the machine was booted up.
    pub on_boot_sec: Vec<String>,
    /// OnStartupSec= — relative to when the service manager was first started.
    pub on_startup_sec: Vec<String>,
    /// OnUnitActiveSec= — relative to when the unit the timer activates was last activated.
    pub on_unit_active_sec: Vec<String>,
    /// OnUnitInactiveSec= — relative to when the unit the timer activates was last deactivated.
    pub on_unit_inactive_sec: Vec<String>,
    /// OnCalendar= — defines realtime (wallclock) calendar event expressions.
    pub on_calendar: Vec<String>,
    /// AccuracySec= — accuracy of the timer, default 1min.
    pub accuracy_sec: Option<String>,
    /// RandomizedDelaySec= — random delay added on top of the timer.
    pub randomized_delay_sec: Option<String>,
    /// FixedRandomDelay= — if true, the random delay is stable across reboots.
    pub fixed_random_delay: bool,
    /// Persistent= — if true, missed runs are triggered immediately on boot.
    pub persistent: bool,
    /// WakeSystem= — if true, wake the system from suspend to fire the timer.
    pub wake_system: bool,
    /// RemainAfterElapse= — if true, timer stays loaded after elapsing (default true).
    pub remain_after_elapse: bool,
    /// OnClockChange= — if true, the timer is triggered when the system clock
    /// jumps relative to the monotonic clock (e.g. DST change, NTP correction).
    /// See systemd.timer(5).
    pub on_clock_change: bool,
    /// OnTimezoneChange= — if true, the timer is triggered when the system
    /// timezone changes. See systemd.timer(5).
    pub on_timezone_change: bool,
    /// Unit= — the unit to activate when the timer elapses (defaults to same-name .service).
    pub unit: Option<String>,
}

/// A parsed condition from the [Unit] section.
/// Systemd supports many condition types; we implement the most common ones.
#[derive(Clone, Debug)]
pub enum UnitCondition {
    /// ConditionPathExists=/some/path (true if path exists)
    /// ConditionPathExists=!/some/path (true if path does NOT exist)
    PathExists { path: String, negate: bool },
    /// ConditionPathExistsGlob=/some/glob* (true if any path matches the glob)
    /// ConditionPathExistsGlob=!/some/glob* (true if NO path matches the glob)
    PathExistsGlob { pattern: String, negate: bool },
    /// ConditionPathIsDirectory=/some/path
    /// ConditionPathIsDirectory=!/some/path
    PathIsDirectory { path: String, negate: bool },
    /// ConditionVirtualization=yes|no|vm|container|<tech-name>
    /// ConditionVirtualization=!<value> (negated)
    /// Checks whether the system runs in a virtualized environment.
    /// See systemd.unit(5).
    Virtualization { value: String, negate: bool },
    /// ConditionCapability=CAP_NET_ADMIN (true if capability is in bounding set)
    /// ConditionCapability=!CAP_SYS_ADMIN (true if capability is NOT in bounding set)
    /// Checks whether a given capability exists in the capability bounding set
    /// of the service manager (PID 1). See systemd.unit(5).
    Capability { capability: String, negate: bool },
    /// ConditionFirstBoot=yes (true if this is the first boot of the system)
    /// ConditionFirstBoot=!yes (true if this is NOT the first boot)
    /// Checks whether the system is booting for the first time (i.e.
    /// /etc/machine-id does not yet exist). See systemd.unit(5).
    FirstBoot { value: bool, negate: bool },
    /// ConditionFileNotEmpty=/some/path (true if path exists as a regular file with size > 0)
    /// ConditionFileNotEmpty=!/some/path (true if path does NOT exist or is empty)
    /// Checks whether the specified path exists as a regular file and has
    /// a non-zero size. See systemd.unit(5).
    FileNotEmpty { path: String, negate: bool },
    /// ConditionFileIsExecutable=/some/path (true if path exists and is executable)
    /// ConditionFileIsExecutable=!/some/path (true if path does NOT exist or is not executable)
    /// Checks whether the specified path exists as a regular file and has
    /// at least one execute bit set. See systemd.unit(5).
    FileIsExecutable { path: String, negate: bool },
    /// ConditionKernelModuleLoaded=module_name (true if the kernel module is loaded)
    /// ConditionKernelModuleLoaded=!module_name (true if the kernel module is NOT loaded)
    /// Checks whether the specified kernel module is currently loaded.
    /// On Linux this is determined by reading /proc/modules. See systemd.unit(5).
    KernelModuleLoaded { module: String, negate: bool },
    /// ConditionKernelCommandLine=option (true if the kernel command line contains the option)
    /// ConditionKernelCommandLine=!option (true if the kernel command line does NOT contain the option)
    /// The argument is either a single word (checked as a standalone parameter or as the
    /// left-hand side of an assignment) or a key=value assignment (checked for an exact match).
    /// Reads from /proc/cmdline (or /proc/1/cmdline in containers). See systemd.unit(5).
    KernelCommandLine { argument: String, negate: bool },
    /// ConditionDirectoryNotEmpty=/some/path (true if path exists as a directory and is not empty)
    /// ConditionDirectoryNotEmpty=!/some/path (true if path does NOT exist, is not a directory, or is empty)
    /// Checks whether the specified path exists, is a directory, and contains
    /// at least one entry (besides "." and ".."). See systemd.unit(5).
    DirectoryNotEmpty { path: String, negate: bool },
    /// ConditionPathIsReadWrite=/some/path (true if path exists on a read-write filesystem)
    /// ConditionPathIsReadWrite=!/some/path (true if path does NOT exist or is on a read-only filesystem)
    /// Checks whether the specified path exists and is on a read-write mounted
    /// filesystem (i.e. not mounted read-only). Uses access(2) with W_OK.
    /// See systemd.unit(5).
    PathIsReadWrite { path: String, negate: bool },
    /// ConditionControlGroupController=controller (true if the cgroup controller is available)
    /// ConditionControlGroupController=!controller (true if the cgroup controller is NOT available)
    /// Checks whether a given cgroup controller (e.g. `cpu`, `memory`, `io`,
    /// `pids`) is available for use in the system's cgroup hierarchy.
    /// On cgroupv2 this reads /sys/fs/cgroup/cgroup.controllers; on cgroupv1
    /// it reads /proc/cgroups. The special value `v2` checks whether the
    /// unified (cgroupv2) hierarchy is in use. See systemd.unit(5).
    ControlGroupController { controller: String, negate: bool },
    /// ConditionNeedsUpdate=/etc (true if /etc is older than /usr)
    /// ConditionNeedsUpdate=!/var (true if /var does NOT need updating)
    /// Checks whether the specified directory needs an update because /usr
    /// has been modified more recently. Takes an absolute path as argument.
    /// Commonly used to trigger post-upgrade configuration updates.
    /// See systemd.unit(5).
    NeedsUpdate { path: String, negate: bool },
    /// ConditionPathIsMountPoint=/some/path (true if path is a mount point)
    /// ConditionPathIsMountPoint=!/some/path (true if path is NOT a mount point)
    /// Checks whether the specified path exists and is a mount point (i.e.
    /// a different filesystem is mounted there compared to the parent directory).
    /// On Linux this is determined by comparing st_dev of the path and its
    /// parent. See systemd.unit(5).
    PathIsMountPoint { path: String, negate: bool },
    /// ConditionSecurity=selinux (true if SELinux is enabled)
    /// ConditionSecurity=!apparmor (true if AppArmor is NOT enabled)
    /// Checks whether a given security technology is enabled on the system.
    /// Known values include: selinux, apparmor, tomoyo, smack, ima, audit,
    /// uefi-secureboot, tpm2, cvm, measured-uki. Multiple directives
    /// accumulate (all must be satisfied). See systemd.unit(5).
    Security { technology: String, negate: bool },
    /// ConditionACPower=true (true if system is on AC power)
    /// ConditionACPower=!true (true if system is NOT on AC power)
    /// Checks whether the system is currently on AC (mains) power.
    /// Reads from /sys/class/power_supply/ looking for an online AC adapter.
    /// See systemd.unit(5).
    ACPower { value: bool, negate: bool },
    /// ConditionArchitecture=x86-64 (true if running on x86-64)
    /// ConditionArchitecture=!arm64 (true if NOT running on arm64)
    /// Checks whether the system is running on a specific CPU architecture.
    /// Uses the native pointer width and target arch to determine the
    /// architecture. Known values: x86, x86-64, arm, arm64, ia64, ppc,
    /// ppc-le, ppc64, ppc64-le, s390, s390x, mips, mips-le, mips64,
    /// mips64-le, alpha, arc, loongarch64, riscv32, riscv64, sh, sparc,
    /// sparc64, native. See systemd.unit(5).
    Architecture { arch: String, negate: bool },
    /// ConditionEnvironment=VAR (true if VAR is set)
    /// ConditionEnvironment=VAR=VALUE (true if VAR equals VALUE)
    /// ConditionEnvironment=!VAR (true if VAR is NOT set)
    /// Checks the manager's environment block for the specified variable.
    /// See systemd.unit(5).
    Environment { expression: String, negate: bool },
    /// ConditionFirmware=uefi (true if running on UEFI firmware)
    /// ConditionFirmware=device-tree (true if running with device-tree)
    /// ConditionFirmware=device-tree-compatible(value) (true if the FDT
    /// compatible string matches). See systemd.unit(5).
    Firmware { value: String, negate: bool },
    /// ConditionHost=hostname (true if hostname matches)
    /// ConditionHost=machine-id (true if machine-id matches, if it looks like a 128-bit hex ID)
    /// ConditionHost=!hostname (true if hostname does NOT match)
    /// Checks the system hostname or machine-id. See systemd.unit(5).
    Host { value: String, negate: bool },
    /// ConditionMemory=512M (true if physical memory >= 512 MiB)
    /// ConditionMemory=!1G (true if physical memory < 1 GiB)
    /// Checks whether the system has at least the specified amount of
    /// physical memory. Supports K, M, G, T suffixes. See systemd.unit(5).
    Memory { value: String, negate: bool },
    /// ConditionCPUFeature=sse4_2 (true if CPU supports SSE 4.2)
    /// ConditionCPUFeature=!avx2 (true if CPU does NOT support AVX2)
    /// Checks whether a specific CPU feature flag is available.
    /// Reads from /proc/cpuinfo "flags" line. See systemd.unit(5).
    CPUFeature { feature: String, negate: bool },
    /// ConditionCPUs=4 (true if system has >= 4 CPUs)
    /// ConditionCPUs=!1 (true if system does NOT have exactly 1 CPU)
    /// Checks the number of online CPUs. Supports comparison operators
    /// (>=, >, <=, <, =, !=) and range expressions. See systemd.unit(5).
    CPUs { expression: String, negate: bool },
    /// ConditionOSRelease=ID=fedora (true if os-release ID equals "fedora")
    /// ConditionOSRelease=!ID=ubuntu (true if os-release ID is NOT "ubuntu")
    /// Checks fields in /etc/os-release or /usr/lib/os-release.
    /// Supports = (equal), != (not equal), <, <=, >, >= comparisons
    /// and fnmatch-style glob patterns. See systemd.unit(5).
    OSRelease { expression: String, negate: bool },
    /// ConditionPathIsEncrypted=/some/path (true if path is on an encrypted volume)
    /// ConditionPathIsEncrypted=!/some/path (true if path is NOT on an encrypted volume)
    /// Checks whether the specified path resides on an encrypted block device
    /// (e.g. dm-crypt/LUKS). Inspects /sys/dev/block/ for dm-crypt attribute.
    /// See systemd.unit(5).
    PathIsEncrypted { path: String, negate: bool },
    /// ConditionPathIsSymbolicLink=/some/path (true if path is a symbolic link)
    /// ConditionPathIsSymbolicLink=!/some/path (true if path is NOT a symbolic link)
    /// Checks whether the specified path exists and is a symbolic link.
    /// See systemd.unit(5).
    PathIsSymbolicLink { path: String, negate: bool },
    /// ConditionUser=root (true if running as root)
    /// ConditionUser=0 (true if running as UID 0)
    /// ConditionUser=!nobody (true if NOT running as nobody)
    /// ConditionUser=@system (true if running as a system user, UID < 1000)
    /// Checks the user identity of the service manager. See systemd.unit(5).
    User { value: String, negate: bool },
    /// ConditionGroup=wheel (true if running with group wheel)
    /// ConditionGroup=0 (true if running with GID 0)
    /// ConditionGroup=!nogroup (true if NOT running with group nogroup)
    /// Checks the group identity of the service manager (primary or supplementary).
    /// See systemd.unit(5).
    Group { value: String, negate: bool },
}

/// The kind of virtualization detected (VM or container).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VirtKind {
    Vm,
    Container,
}

/// Detected virtualization: the technology name and its kind.
#[derive(Clone, Debug)]
pub struct DetectedVirt {
    pub name: String,
    pub kind: VirtKind,
}

/// Map a Linux capability name (e.g. "CAP_NET_ADMIN") to its bit number.
/// Returns `None` if the name is not recognized.
fn capability_name_to_bit(name: &str) -> Option<u64> {
    match name.to_uppercase().as_str() {
        "CAP_CHOWN" => Some(0),
        "CAP_DAC_OVERRIDE" => Some(1),
        "CAP_DAC_READ_SEARCH" => Some(2),
        "CAP_FOWNER" => Some(3),
        "CAP_FSETID" => Some(4),
        "CAP_KILL" => Some(5),
        "CAP_SETGID" => Some(6),
        "CAP_SETUID" => Some(7),
        "CAP_SETPCAP" => Some(8),
        "CAP_LINUX_IMMUTABLE" => Some(9),
        "CAP_NET_BIND_SERVICE" => Some(10),
        "CAP_NET_BROADCAST" => Some(11),
        "CAP_NET_ADMIN" => Some(12),
        "CAP_NET_RAW" => Some(13),
        "CAP_IPC_LOCK" => Some(14),
        "CAP_IPC_OWNER" => Some(15),
        "CAP_SYS_MODULE" => Some(16),
        "CAP_SYS_RAWIO" => Some(17),
        "CAP_SYS_CHROOT" => Some(18),
        "CAP_SYS_PTRACE" => Some(19),
        "CAP_SYS_PACCT" => Some(20),
        "CAP_SYS_ADMIN" => Some(21),
        "CAP_SYS_BOOT" => Some(22),
        "CAP_SYS_NICE" => Some(23),
        "CAP_SYS_RESOURCE" => Some(24),
        "CAP_SYS_TIME" => Some(25),
        "CAP_SYS_TTY_CONFIG" => Some(26),
        "CAP_MKNOD" => Some(27),
        "CAP_LEASE" => Some(28),
        "CAP_AUDIT_WRITE" => Some(29),
        "CAP_AUDIT_CONTROL" => Some(30),
        "CAP_SETFCAP" => Some(31),
        "CAP_MAC_OVERRIDE" => Some(32),
        "CAP_MAC_ADMIN" => Some(33),
        "CAP_SYSLOG" => Some(34),
        "CAP_WAKE_ALARM" => Some(35),
        "CAP_BLOCK_SUSPEND" => Some(36),
        "CAP_AUDIT_READ" => Some(37),
        "CAP_PERFMON" => Some(38),
        "CAP_BPF" => Some(39),
        "CAP_CHECKPOINT_RESTORE" => Some(40),
        _ => None,
    }
}

/// Check whether a capability (by bit number) is present in the capability
/// bounding set of the service manager. Reads `/proc/self/status` and parses
/// the `CapBnd:` hex field.
fn capability_in_bounding_set(bit: u64) -> bool {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some(hex) = line.strip_prefix("CapBnd:") {
                let hex = hex.trim().trim_start_matches("0x");
                if let Ok(mask) = u64::from_str_radix(hex, 16) {
                    return (mask & (1u64 << bit)) != 0;
                }
            }
        }
    }
    // If we can't read the status, conservatively assume the capability is present
    // (matches systemd behavior for privileged service managers).
    true
}

/// Detect whether the system is running inside a virtualized environment.
///
/// Returns `Some(DetectedVirt)` with the technology name and kind, or `None`
/// if running on bare metal. The detection mirrors a subset of the logic from
/// `systemd-detect-virt(1)`.
fn detect_virtualization() -> Option<DetectedVirt> {
    // --- Container detection (higher priority) ---

    // Check /.dockerenv
    if std::path::Path::new("/.dockerenv").exists() {
        return Some(DetectedVirt {
            name: "docker".to_owned(),
            kind: VirtKind::Container,
        });
    }

    // Check /run/.containerenv (podman / buildah)
    if std::path::Path::new("/run/.containerenv").exists() {
        return Some(DetectedVirt {
            name: "podman".to_owned(),
            kind: VirtKind::Container,
        });
    }

    // Check the "container" environment variable of PID 1
    if let Ok(environ) = std::fs::read("/proc/1/environ") {
        // environ is NUL-separated KEY=VALUE pairs
        for entry in environ.split(|&b| b == 0) {
            if let Ok(s) = std::str::from_utf8(entry)
                && let Some(val) = s.strip_prefix("container=")
            {
                let name = match val {
                    "systemd-nspawn" => "systemd-nspawn",
                    "lxc" => "lxc",
                    "lxc-libvirt" => "lxc-libvirt",
                    "docker" => "docker",
                    "podman" => "podman",
                    "rkt" => "rkt",
                    "wsl" => "wsl",
                    "proot" => "proot",
                    "pouch" => "pouch",
                    "oci" => "docker", // generic OCI → docker
                    other => other,
                };
                return Some(DetectedVirt {
                    name: name.to_owned(),
                    kind: VirtKind::Container,
                });
            }
        }
    }

    // Check cgroup for container hints
    if let Ok(cgroup) = std::fs::read_to_string("/proc/1/cgroup") {
        let lower = cgroup.to_lowercase();
        if lower.contains("/docker/") || lower.contains("/docker-") {
            return Some(DetectedVirt {
                name: "docker".to_owned(),
                kind: VirtKind::Container,
            });
        }
        if lower.contains("/lxc/") || lower.contains("/lxc.payload") {
            return Some(DetectedVirt {
                name: "lxc".to_owned(),
                kind: VirtKind::Container,
            });
        }
    }

    // --- VM detection ---

    // Check /proc/xen for Xen guest
    if std::path::Path::new("/proc/xen").exists() {
        // If capabilities file contains "control_d" we are dom0 (host), not guest
        if let Ok(caps) = std::fs::read_to_string("/proc/xen/capabilities") {
            if !caps.contains("control_d") {
                return Some(DetectedVirt {
                    name: "xen".to_owned(),
                    kind: VirtKind::Vm,
                });
            }
        } else {
            return Some(DetectedVirt {
                name: "xen".to_owned(),
                kind: VirtKind::Vm,
            });
        }
    }

    // Check /sys/hypervisor/type
    if let Ok(htype) = std::fs::read_to_string("/sys/hypervisor/type") {
        let htype = htype.trim().to_lowercase();
        if htype == "xen" {
            return Some(DetectedVirt {
                name: "xen".to_owned(),
                kind: VirtKind::Vm,
            });
        }
    }

    // DMI-based detection: sys_vendor, product_name, board_vendor
    let dmi_files = [
        "/sys/class/dmi/id/sys_vendor",
        "/sys/class/dmi/id/product_name",
        "/sys/class/dmi/id/board_vendor",
    ];
    for path in &dmi_files {
        if let Ok(content) = std::fs::read_to_string(path) {
            let lower = content.trim().to_lowercase();
            if lower.contains("qemu") || lower.contains("kvm") {
                return Some(DetectedVirt {
                    name: "kvm".to_owned(),
                    kind: VirtKind::Vm,
                });
            }
            if lower.contains("vmware") {
                return Some(DetectedVirt {
                    name: "vmware".to_owned(),
                    kind: VirtKind::Vm,
                });
            }
            if lower.contains("microsoft") || lower.contains("hyper-v") {
                return Some(DetectedVirt {
                    name: "microsoft".to_owned(),
                    kind: VirtKind::Vm,
                });
            }
            if lower.contains("virtualbox") || lower.contains("oracle") {
                return Some(DetectedVirt {
                    name: "oracle".to_owned(),
                    kind: VirtKind::Vm,
                });
            }
            if lower.contains("parallels") {
                return Some(DetectedVirt {
                    name: "parallels".to_owned(),
                    kind: VirtKind::Vm,
                });
            }
            if lower.contains("bhyve") {
                return Some(DetectedVirt {
                    name: "bhyve".to_owned(),
                    kind: VirtKind::Vm,
                });
            }
            if lower.contains("bochs") {
                return Some(DetectedVirt {
                    name: "bochs".to_owned(),
                    kind: VirtKind::Vm,
                });
            }
            if lower.contains("apple virtualization") {
                return Some(DetectedVirt {
                    name: "apple".to_owned(),
                    kind: VirtKind::Vm,
                });
            }
            if lower.contains("google") {
                return Some(DetectedVirt {
                    name: "google".to_owned(),
                    kind: VirtKind::Vm,
                });
            }
            if lower.contains("acrn") {
                return Some(DetectedVirt {
                    name: "acrn".to_owned(),
                    kind: VirtKind::Vm,
                });
            }
        }
    }

    // Check /proc/cpuinfo for hypervisor flag
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        for line in cpuinfo.lines() {
            if (line.starts_with("flags") || line.starts_with("Features"))
                && (line.contains(" hypervisor") || line.contains("\thypervisor"))
            {
                // Generic VM detected via CPUID hypervisor bit
                return Some(DetectedVirt {
                    name: "vm-other".to_owned(),
                    kind: VirtKind::Vm,
                });
            }
        }
    }

    // Check for UML (User-Mode Linux)
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        for line in cpuinfo.lines() {
            if line.starts_with("vendor_id") && line.contains("User Mode Linux") {
                return Some(DetectedVirt {
                    name: "uml".to_owned(),
                    kind: VirtKind::Vm,
                });
            }
        }
    }

    // Check for WSL (Windows Subsystem for Linux) via /proc/version
    if let Ok(version) = std::fs::read_to_string("/proc/version") {
        let lower = version.to_lowercase();
        if lower.contains("microsoft") || lower.contains("wsl") {
            return Some(DetectedVirt {
                name: "wsl".to_owned(),
                kind: VirtKind::Container,
            });
        }
    }

    None
}

impl UnitCondition {
    /// Evaluate the condition. Returns true if the condition is met.
    pub fn check(&self) -> bool {
        match self {
            UnitCondition::PathExists { path, negate } => {
                let exists = std::path::Path::new(path).exists();
                if *negate { !exists } else { exists }
            }
            UnitCondition::PathExistsGlob { pattern, negate } => {
                let matched = condition_glob_match_any(pattern);
                if *negate { !matched } else { matched }
            }
            UnitCondition::PathIsDirectory { path, negate } => {
                let is_dir = std::path::Path::new(path).is_dir();
                if *negate { !is_dir } else { is_dir }
            }
            UnitCondition::Virtualization { value, negate } => {
                let detected = detect_virtualization();
                let result = match value.as_str() {
                    // Boolean: any virtualization at all?
                    "yes" | "true" | "1" => detected.is_some(),
                    "no" | "false" | "0" => detected.is_none(),
                    // Category checks
                    "vm" => detected.as_ref().is_some_and(|d| d.kind == VirtKind::Vm),
                    "container" => detected
                        .as_ref()
                        .is_some_and(|d| d.kind == VirtKind::Container),
                    // Specific technology name
                    tech => detected.as_ref().is_some_and(|d| d.name == tech),
                };
                if *negate { !result } else { result }
            }
            UnitCondition::Capability { capability, negate } => {
                let result = match capability_name_to_bit(capability) {
                    Some(bit) => capability_in_bounding_set(bit),
                    None => {
                        // Unknown capability name — treat as not present
                        trace!(
                            "Unknown capability name in ConditionCapability: {}",
                            capability
                        );
                        false
                    }
                };
                if *negate { !result } else { result }
            }
            UnitCondition::FirstBoot { value, negate } => {
                // systemd considers it "first boot" when /etc/machine-id
                // does not exist or is empty (uninitialized).
                let is_first_boot = match std::fs::metadata("/etc/machine-id") {
                    Ok(meta) => meta.len() == 0,
                    Err(_) => true, // file doesn't exist → first boot
                };
                let result = if *value {
                    is_first_boot
                } else {
                    !is_first_boot
                };
                if *negate { !result } else { result }
            }
            UnitCondition::FileIsExecutable { path, negate } => {
                use std::os::unix::fs::PermissionsExt;
                let is_executable = match std::fs::metadata(path) {
                    Ok(meta) => {
                        // Must be a regular file with at least one execute bit
                        meta.is_file() && (meta.permissions().mode() & 0o111 != 0)
                    }
                    Err(_) => false,
                };
                if *negate {
                    !is_executable
                } else {
                    is_executable
                }
            }
            UnitCondition::FileNotEmpty { path, negate } => {
                let is_non_empty = match std::fs::metadata(path) {
                    Ok(meta) => {
                        // Must be a regular file with size > 0
                        meta.is_file() && meta.len() > 0
                    }
                    Err(_) => false,
                };
                if *negate { !is_non_empty } else { is_non_empty }
            }
            UnitCondition::KernelModuleLoaded { module, negate } => {
                let is_loaded = match std::fs::read_to_string("/proc/modules") {
                    Ok(contents) => {
                        // Each line in /proc/modules starts with the module name
                        // followed by a space. We check for an exact module name match.
                        contents
                            .lines()
                            .any(|line| line.split_whitespace().next() == Some(module.as_str()))
                    }
                    Err(_) => false,
                };
                if *negate { !is_loaded } else { is_loaded }
            }
            UnitCondition::DirectoryNotEmpty { path, negate } => {
                let is_non_empty = match std::fs::read_dir(path) {
                    Ok(mut entries) => {
                        // A directory is "not empty" if it contains at least one entry
                        entries.next().is_some()
                    }
                    Err(_) => false, // doesn't exist or isn't a directory
                };
                if *negate { !is_non_empty } else { is_non_empty }
            }
            UnitCondition::KernelCommandLine { argument, negate } => {
                let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
                let result = if argument.contains('=') {
                    // Exact key=value match: look for the whole assignment as a
                    // whitespace-delimited token on the command line.
                    cmdline.split_whitespace().any(|token| token == argument)
                } else {
                    // Single word: match either the bare word or as the key
                    // part of a key=value pair.
                    cmdline.split_whitespace().any(|token| {
                        token == argument
                            || token
                                .split_once('=')
                                .is_some_and(|(key, _)| key == argument)
                    })
                };
                if *negate { !result } else { result }
            }
            UnitCondition::PathIsReadWrite { path, negate } => {
                // Check whether the path is on a read-write filesystem.
                // We use access(W_OK) which checks whether the process could
                // write to the path — if the filesystem is read-only this
                // will report false. For directories we check the directory
                // itself; for files we check the file.
                let is_rw = match nix::unistd::access(path.as_str(), nix::unistd::AccessFlags::W_OK)
                {
                    Ok(()) => true,
                    Err(_) => false,
                };
                if *negate { !is_rw } else { is_rw }
            }
            UnitCondition::ControlGroupController { controller, negate } => {
                let result = if controller == "v2" {
                    // Special value: check whether the unified (cgroupv2) hierarchy is in use.
                    // cgroupv2 mounts a "cgroup2" filesystem at /sys/fs/cgroup.
                    std::path::Path::new("/sys/fs/cgroup/cgroup.controllers").exists()
                } else {
                    // First try cgroupv2: read /sys/fs/cgroup/cgroup.controllers
                    let v2_available = std::fs::read_to_string("/sys/fs/cgroup/cgroup.controllers")
                        .map(|contents| {
                            contents
                                .split_whitespace()
                                .any(|c| c == controller.as_str())
                        })
                        .unwrap_or(false);
                    if v2_available {
                        true
                    } else {
                        // Fallback to cgroupv1: read /proc/cgroups
                        // Format: "controller\thierarchy\tnum_cgroups\tenabled"
                        std::fs::read_to_string("/proc/cgroups")
                            .map(|contents| {
                                contents.lines().any(|line| {
                                    if line.starts_with('#') {
                                        return false;
                                    }
                                    let mut fields = line.split_whitespace();
                                    let name = fields.next().unwrap_or("");
                                    // The 4th field is "enabled" (1 = yes)
                                    let enabled = fields.nth(2).unwrap_or("0");
                                    name == controller.as_str() && enabled == "1"
                                })
                            })
                            .unwrap_or(false)
                    }
                };
                if *negate { !result } else { result }
            }
            UnitCondition::NeedsUpdate { path, negate } => {
                // ConditionNeedsUpdate= checks whether the specified directory
                // needs an update because /usr has been modified more recently.
                // Compare mtime of the path against mtime of /usr.
                let needs_update = match (
                    std::fs::metadata(path).and_then(|m| m.modified()),
                    std::fs::metadata("/usr").and_then(|m| m.modified()),
                ) {
                    (Ok(path_mtime), Ok(usr_mtime)) => usr_mtime > path_mtime,
                    // If either path doesn't exist or we can't stat it,
                    // treat it as "needs update" (matching systemd behavior:
                    // missing stamp file means update is needed).
                    (Err(_), Ok(_)) => true,
                    _ => false,
                };
                if *negate { !needs_update } else { needs_update }
            }
            UnitCondition::PathIsMountPoint { path, negate } => {
                // ConditionPathIsMountPoint= checks whether the specified path
                // is a mount point. On Linux, a path is a mount point if its
                // st_dev differs from its parent directory's st_dev, or if it
                // is the filesystem root (path == parent).
                use std::os::unix::fs::MetadataExt;
                let is_mount_point = std::fs::metadata(path)
                    .and_then(|meta| {
                        let path = std::path::Path::new(path);
                        let parent = path.parent().unwrap_or(path);
                        let parent_meta = std::fs::metadata(parent)?;
                        // A path is a mount point if its device ID differs from
                        // its parent's, or if it IS the root (parent == self).
                        Ok(meta.dev() != parent_meta.dev() || path == parent)
                    })
                    .unwrap_or(false);
                if *negate {
                    !is_mount_point
                } else {
                    is_mount_point
                }
            }
            UnitCondition::Security { technology, negate } => {
                let enabled = match technology.as_str() {
                    // SELinux: check if the SELinux filesystem is mounted
                    "selinux" => std::path::Path::new("/sys/fs/selinux").exists(),
                    // AppArmor: check if the kernel module is loaded
                    "apparmor" => std::path::Path::new("/sys/module/apparmor").exists(),
                    // TOMOYO: check if the TOMOYO security interface exists
                    "tomoyo" => std::path::Path::new("/sys/kernel/security/tomoyo").exists(),
                    // SMACK: check if the SMACK filesystem is mounted
                    "smack" => std::path::Path::new("/sys/fs/smackfs").exists(),
                    // IMA: check if the IMA security interface exists
                    "ima" => std::path::Path::new("/sys/kernel/security/ima").exists(),
                    // Audit: check if the kernel audit subsystem is available
                    "audit" => {
                        std::path::Path::new("/proc/sys/kernel/audit_arch").exists()
                            || std::path::Path::new("/sys/kernel/security/audit").exists()
                    }
                    // UEFI Secure Boot: check if EFI is available and Secure Boot is on
                    "uefi-secureboot" => {
                        // Look for any SecureBoot EFI variable
                        std::path::Path::new("/sys/firmware/efi").exists()
                            && std::fs::read_dir("/sys/firmware/efi/efivars")
                                .map(|entries| {
                                    entries.filter_map(|e| e.ok()).any(|e| {
                                        e.file_name().to_string_lossy().starts_with("SecureBoot-")
                                    })
                                })
                                .unwrap_or(false)
                    }
                    // TPM2: check if a TPM 2.0 resource manager device exists
                    "tpm2" => std::path::Path::new("/sys/class/tpmrm/tpmrm0").exists(),
                    // CVM (Confidential Virtual Machine): check for AMD SEV or Intel TDX
                    "cvm" => {
                        std::path::Path::new("/sys/firmware/acpi/tables/CCEL").exists()
                            || std::path::Path::new("/dev/sev").exists()
                            || std::path::Path::new("/dev/tdx-guest").exists()
                    }
                    // measured-uki: check for UKI measurement markers
                    "measured-uki" => {
                        // Check for the presence of the StubPcrKernelImage EFI variable
                        // which indicates a measured Unified Kernel Image boot
                        std::fs::read_dir("/sys/firmware/efi/efivars")
                            .map(|entries| {
                                entries.filter_map(|e| e.ok()).any(|e| {
                                    e.file_name()
                                        .to_string_lossy()
                                        .starts_with("StubPcrKernelImage-")
                                })
                            })
                            .unwrap_or(false)
                    }
                    other => {
                        trace!(
                            "Unknown security technology in ConditionSecurity: {}",
                            other
                        );
                        false
                    }
                };
                if *negate { !enabled } else { enabled }
            }
            UnitCondition::ACPower { value, negate } => {
                let on_ac = check_ac_power();
                let result = if *value { on_ac } else { !on_ac };
                if *negate { !result } else { result }
            }
            UnitCondition::Architecture { arch, negate } => {
                let current = detect_architecture();
                let result = if arch == "native" {
                    // "native" always matches the current architecture
                    true
                } else {
                    current.eq_ignore_ascii_case(arch)
                };
                if *negate { !result } else { result }
            }
            UnitCondition::Environment { expression, negate } => {
                let result = if let Some((key, expected_value)) = expression.split_once('=') {
                    // KEY=VALUE form: check if env var equals the expected value
                    std::env::var(key).is_ok_and(|v| v == expected_value)
                } else {
                    // KEY form: check if env var is set (any value)
                    std::env::var(expression).is_ok()
                };
                if *negate { !result } else { result }
            }
            UnitCondition::Firmware { value, negate } => {
                let result = match value.as_str() {
                    "uefi" => {
                        // Check if the system booted via UEFI
                        std::path::Path::new("/sys/firmware/efi").exists()
                    }
                    "device-tree" => {
                        // Check if the system has a device tree
                        std::path::Path::new("/sys/firmware/devicetree/base").exists()
                    }
                    v if v.starts_with("device-tree-compatible(") && v.ends_with(')') => {
                        // device-tree-compatible(value) — check /sys/firmware/devicetree/base/compatible
                        let inner = &v["device-tree-compatible(".len()..v.len() - 1];
                        match std::fs::read("/sys/firmware/devicetree/base/compatible") {
                            Ok(data) => {
                                // The compatible property is a null-separated list of strings
                                data.split(|&b| b == 0).filter(|s| !s.is_empty()).any(|s| {
                                    std::str::from_utf8(s).is_ok_and(|compat| compat == inner)
                                })
                            }
                            Err(_) => false,
                        }
                    }
                    other => {
                        trace!("Unknown firmware type in ConditionFirmware: {}", other);
                        false
                    }
                };
                if *negate { !result } else { result }
            }
            UnitCondition::Host { value, negate } => {
                let result = if looks_like_machine_id(value) {
                    // Looks like a machine-id (128-bit hex string) — compare against /etc/machine-id
                    std::fs::read_to_string("/etc/machine-id")
                        .map(|id| id.trim() == value)
                        .unwrap_or(false)
                } else {
                    // Compare against the system hostname
                    nix::unistd::gethostname()
                        .ok()
                        .and_then(|h| h.into_string().ok())
                        .is_some_and(|hostname| hostname == *value)
                };
                if *negate { !result } else { result }
            }
            UnitCondition::Memory { value, negate } => {
                let result = match parse_memory_condition(value) {
                    Some(threshold_bytes) => {
                        let total = read_total_memory();
                        total >= threshold_bytes
                    }
                    None => {
                        trace!("Failed to parse ConditionMemory value: {}", value);
                        false
                    }
                };
                if *negate { !result } else { result }
            }
            UnitCondition::CPUFeature { feature, negate } => {
                let result = check_cpu_feature(feature);
                if *negate { !result } else { result }
            }
            UnitCondition::CPUs { expression, negate } => {
                let result = check_cpus_condition(expression);
                if *negate { !result } else { result }
            }
            UnitCondition::OSRelease { expression, negate } => {
                let result = check_os_release_condition(expression);
                if *negate { !result } else { result }
            }
            UnitCondition::PathIsEncrypted { path, negate } => {
                let encrypted = check_path_encrypted(path);
                if *negate { !encrypted } else { encrypted }
            }
            UnitCondition::PathIsSymbolicLink { path, negate } => {
                let is_symlink = std::fs::symlink_metadata(path)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                if *negate { !is_symlink } else { is_symlink }
            }
            UnitCondition::User { value, negate } => {
                let result = check_user_condition(value);
                if *negate { !result } else { result }
            }
            UnitCondition::Group { value, negate } => {
                let result = check_group_condition(value);
                if *negate { !result } else { result }
            }
        }
    }
}

/// Check if any filesystem path matches a glob pattern.
/// Supports `*` and `?` wildcards in the filename component.
fn condition_glob_match_any(pattern: &str) -> bool {
    let path = std::path::Path::new(pattern);
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => std::path::Path::new("/"),
    };
    let file_pattern = match path.file_name() {
        Some(f) => f.to_string_lossy().to_string(),
        None => return false,
    };
    let entries = match std::fs::read_dir(parent) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if crate::link_config::glob_match(&file_pattern, &name) {
            return true;
        }
    }
    false
}

/// Check whether the system is currently on AC (mains) power.
/// Scans /sys/class/power_supply/ for an online AC/mains supply.
fn check_ac_power() -> bool {
    let ps_dir = std::path::Path::new("/sys/class/power_supply");
    let entries = match std::fs::read_dir(ps_dir) {
        Ok(e) => e,
        Err(_) => return true, // If we can't read, assume AC (safe default)
    };
    // If there are no power supply entries at all, assume AC
    let mut found_any = false;
    for entry in entries.flatten() {
        let type_path = entry.path().join("type");
        let supply_type = match std::fs::read_to_string(&type_path) {
            Ok(t) => t.trim().to_lowercase(),
            Err(_) => continue,
        };
        if supply_type == "mains" || supply_type == "ups" {
            found_any = true;
            let online_path = entry.path().join("online");
            if let Ok(online) = std::fs::read_to_string(online_path)
                && online.trim() == "1"
            {
                return true;
            }
        }
    }
    // If we found mains/UPS supplies but none are online, we're on battery
    // If we found no mains/UPS supplies at all, assume AC (desktop/server)
    !found_any
}

/// Detect the CPU architecture of the running system.
/// Returns a string matching systemd's architecture names.
fn detect_architecture() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "x86-64"
    }
    #[cfg(target_arch = "x86")]
    {
        "x86"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "arm64"
    }
    #[cfg(target_arch = "arm")]
    {
        "arm"
    }
    #[cfg(target_arch = "powerpc64")]
    {
        if cfg!(target_endian = "little") {
            "ppc64-le"
        } else {
            "ppc64"
        }
    }
    #[cfg(target_arch = "powerpc")]
    {
        if cfg!(target_endian = "little") {
            "ppc-le"
        } else {
            "ppc"
        }
    }
    #[cfg(target_arch = "s390x")]
    {
        "s390x"
    }
    #[cfg(target_arch = "mips64")]
    {
        if cfg!(target_endian = "little") {
            "mips64-le"
        } else {
            "mips64"
        }
    }
    #[cfg(target_arch = "mips")]
    {
        if cfg!(target_endian = "little") {
            "mips-le"
        } else {
            "mips"
        }
    }
    #[cfg(target_arch = "riscv64")]
    {
        "riscv64"
    }
    #[cfg(target_arch = "riscv32")]
    {
        "riscv32"
    }
    #[cfg(target_arch = "loongarch64")]
    {
        "loongarch64"
    }
    #[cfg(target_arch = "sparc64")]
    {
        "sparc64"
    }
    #[cfg(target_arch = "sparc")]
    {
        "sparc"
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "x86",
        target_arch = "aarch64",
        target_arch = "arm",
        target_arch = "powerpc64",
        target_arch = "powerpc",
        target_arch = "s390x",
        target_arch = "mips64",
        target_arch = "mips",
        target_arch = "riscv64",
        target_arch = "riscv32",
        target_arch = "loongarch64",
        target_arch = "sparc64",
        target_arch = "sparc",
    )))]
    {
        "unknown"
    }
}

/// Check whether a string looks like a 128-bit hex machine-id (32 hex chars).
fn looks_like_machine_id(s: &str) -> bool {
    s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Parse a memory size value for ConditionMemory= (e.g. "512M", "1G", "4096").
/// Returns the value in bytes, or None if parsing fails.
fn parse_memory_condition(value: &str) -> Option<u64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Check for comparison operators at the start (>=, >, <=, <, =)
    let num_str = trimmed
        .strip_prefix(">=")
        .or_else(|| trimmed.strip_prefix('>'))
        .or_else(|| trimmed.strip_prefix("<="))
        .or_else(|| trimmed.strip_prefix('<'))
        .or_else(|| trimmed.strip_prefix('='))
        .unwrap_or(trimmed)
        .trim();

    // Parse number with optional suffix
    let (digits, multiplier) = if let Some(d) = num_str.strip_suffix('T') {
        (d, 1024u64 * 1024 * 1024 * 1024)
    } else if let Some(d) = num_str.strip_suffix('G') {
        (d, 1024u64 * 1024 * 1024)
    } else if let Some(d) = num_str.strip_suffix('M') {
        (d, 1024u64 * 1024)
    } else if let Some(d) = num_str.strip_suffix('K') {
        (d, 1024u64)
    } else {
        (num_str, 1u64)
    };

    digits.trim().parse::<u64>().ok().map(|v| v * multiplier)
}

/// Read total physical memory in bytes from /proc/meminfo.
fn read_total_memory() -> u64 {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|contents| {
            for line in contents.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    // Value is in kB
                    let kb_str = rest.trim().strip_suffix("kB").unwrap_or(rest).trim();
                    return kb_str.parse::<u64>().ok().map(|kb| kb * 1024);
                }
            }
            None
        })
        .unwrap_or(0)
}

/// Check whether a CPU feature flag is present.
/// Reads /proc/cpuinfo and looks in the "flags" (x86), "Features" (arm64),
/// or "features" (s390x) line.
fn check_cpu_feature(feature: &str) -> bool {
    let cpuinfo = match std::fs::read_to_string("/proc/cpuinfo") {
        Ok(c) => c,
        Err(_) => return false,
    };
    let feature_lower = feature.to_lowercase();
    for line in cpuinfo.lines() {
        // x86: "flags", ARM64: "Features", s390x: "features"
        let trimmed = line.trim();
        if let Some(rest) = trimmed
            .strip_prefix("flags")
            .or_else(|| trimmed.strip_prefix("Features"))
            .or_else(|| trimmed.strip_prefix("features"))
            && let Some(flags_str) = rest.strip_prefix(':')
        {
            return flags_str
                .split_whitespace()
                .any(|f| f.to_lowercase() == feature_lower);
        }
    }
    false
}

/// Check the ConditionCPUs= expression against the number of online CPUs.
/// Supports: plain number (>=), or comparison operators (>=N, >N, <=N, <N, =N, !=N).
fn check_cpus_condition(expression: &str) -> bool {
    let trimmed = expression.trim();
    let num_cpus = num_online_cpus();

    if let Some(val) = trimmed.strip_prefix(">=") {
        val.trim().parse::<u32>().is_ok_and(|n| num_cpus >= n)
    } else if let Some(val) = trimmed.strip_prefix('>') {
        val.trim().parse::<u32>().is_ok_and(|n| num_cpus > n)
    } else if let Some(val) = trimmed.strip_prefix("<=") {
        val.trim().parse::<u32>().is_ok_and(|n| num_cpus <= n)
    } else if let Some(val) = trimmed.strip_prefix("!=") {
        val.trim().parse::<u32>().is_ok_and(|n| num_cpus != n)
    } else if let Some(val) = trimmed.strip_prefix('<') {
        val.trim().parse::<u32>().is_ok_and(|n| num_cpus < n)
    } else if let Some(val) = trimmed.strip_prefix('=') {
        val.trim().parse::<u32>().is_ok_and(|n| num_cpus == n)
    } else {
        // Plain number: default is >=
        trimmed.parse::<u32>().is_ok_and(|n| num_cpus >= n)
    }
}

/// Get the number of online CPUs.
fn num_online_cpus() -> u32 {
    std::fs::read_to_string("/sys/devices/system/cpu/online")
        .ok()
        .and_then(|s| parse_cpu_range_count(s.trim()))
        .unwrap_or(1)
}

/// Parse a CPU range string like "0-7" or "0-3,5,7-9" and return the count.
fn parse_cpu_range_count(s: &str) -> Option<u32> {
    let mut count = 0u32;
    for part in s.split(',') {
        let part = part.trim();
        if let Some((start, end)) = part.split_once('-') {
            let start: u32 = start.trim().parse().ok()?;
            let end: u32 = end.trim().parse().ok()?;
            count += end - start + 1;
        } else {
            let _: u32 = part.parse().ok()?;
            count += 1;
        }
    }
    Some(count)
}

/// Check an os-release condition expression.
/// Supports KEY=VALUE (equality), KEY!=VALUE, KEY<VALUE, KEY<=VALUE, KEY>VALUE, KEY>=VALUE.
/// Reads from /etc/os-release or /usr/lib/os-release.
fn check_os_release_condition(expression: &str) -> bool {
    let os_release = read_os_release();

    // Try multi-char operators first, then single-char
    let operators = ["!=", ">=", "<=", "=", ">", "<"];
    for op in &operators {
        if let Some((key, expected)) = expression.split_once(op) {
            let key = key.trim();
            let expected = expected.trim();
            let actual = os_release.get(key).map(|s| s.as_str()).unwrap_or("");
            return match *op {
                "=" => actual == expected,
                "!=" => actual != expected,
                ">=" => actual >= expected,
                "<=" => actual <= expected,
                ">" => actual > expected,
                "<" => actual < expected,
                _ => false,
            };
        }
    }
    false
}

/// Read os-release fields from /etc/os-release or /usr/lib/os-release.
fn read_os_release() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let content = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            // Strip quotes from value
            let value = value.trim();
            let value = if (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''))
            {
                &value[1..value.len() - 1]
            } else {
                value
            };
            map.insert(key.trim().to_string(), value.to_string());
        }
    }
    map
}

/// Check whether a path resides on an encrypted block device (dm-crypt/LUKS).
/// Inspects /sys/dev/block/<major>:<minor>/dm/uuid for a "CRYPT-" prefix.
fn check_path_encrypted(path: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let dev = meta.dev();
    let major = libc::major(dev);
    let minor = libc::minor(dev);
    let dm_uuid_path = format!("/sys/dev/block/{}:{}/dm/uuid", major, minor);
    if let Ok(uuid) = std::fs::read_to_string(dm_uuid_path) {
        return uuid.trim().starts_with("CRYPT-");
    }
    // Also check parent device for partitions
    let slaves_dir = format!("/sys/dev/block/{}:{}/slaves", major, minor);
    if let Ok(entries) = std::fs::read_dir(slaves_dir) {
        for entry in entries.flatten() {
            let uuid_path = entry.path().join("dm/uuid");
            if let Ok(uuid) = std::fs::read_to_string(uuid_path)
                && uuid.trim().starts_with("CRYPT-")
            {
                return true;
            }
        }
    }
    false
}

/// Check the ConditionUser= value against the current user.
/// Values: numeric UID, username, "@system" (UID < 1000).
fn check_user_condition(value: &str) -> bool {
    let uid = nix::unistd::getuid();
    if value == "@system" {
        return uid.as_raw() < 1000;
    }
    // Try as numeric UID first
    if let Ok(expected_uid) = value.parse::<u32>() {
        return uid.as_raw() == expected_uid;
    }
    // Try as username — resolve via /etc/passwd
    match nix::unistd::User::from_name(value) {
        Ok(Some(user)) => uid == user.uid,
        _ => false,
    }
}

/// Check the ConditionGroup= value against the current group(s).
/// Checks primary GID and all supplementary groups.
fn check_group_condition(value: &str) -> bool {
    let gid = nix::unistd::getgid();
    // Try as numeric GID first
    if let Ok(expected_gid) = value.parse::<u32>() {
        if gid.as_raw() == expected_gid {
            return true;
        }
        // Check supplementary groups
        if let Ok(groups) = nix::unistd::getgroups() {
            return groups.iter().any(|g| g.as_raw() == expected_gid);
        }
        return false;
    }
    // Try as group name — resolve via /etc/group
    match nix::unistd::Group::from_name(value) {
        Ok(Some(group)) => {
            if gid == group.gid {
                return true;
            }
            // Check supplementary groups
            if let Ok(groups) = nix::unistd::getgroups() {
                return groups.contains(&group.gid);
            }
            false
        }
        _ => false,
    }
}

/// Action to take when a unit succeeds or fails.
///
/// Matches systemd's `SuccessAction=` / `FailureAction=` settings.
/// See <https://www.freedesktop.org/software/systemd/man/systemd.unit.html#SuccessAction=>.
#[derive(Clone, Eq, PartialEq, Debug, Default)]
pub enum UnitAction {
    /// Do nothing (default).
    #[default]
    None,
    /// Initiate a clean shutdown of the service manager.
    Exit,
    /// Like `Exit`, but without waiting for running jobs to finish.
    ExitForce,
    /// Initiate a clean reboot.
    Reboot,
    /// Reboot immediately, skipping clean shutdown of remaining units.
    RebootForce,
    /// Reboot immediately via `reboot(2)`, skipping all cleanup.
    RebootImmediate,
    /// Initiate a clean poweroff.
    Poweroff,
    /// Poweroff immediately, skipping clean shutdown of remaining units.
    PoweroffForce,
    /// Poweroff immediately via `reboot(2)`, skipping all cleanup.
    PoweroffImmediate,
    /// Initiate a clean halt.
    Halt,
    /// Halt immediately, skipping clean shutdown of remaining units.
    HaltForce,
    /// Halt immediately via `reboot(2)`, skipping all cleanup.
    HaltImmediate,
    /// Initiate a kexec reboot.
    Kexec,
    /// Kexec immediately, skipping clean shutdown of remaining units.
    KexecForce,
    /// Kexec immediately via `reboot(2)`, skipping all cleanup.
    KexecImmediate,
}

/// DevicePolicy= — controls the policy for device access for the unit.
/// Matches systemd's `DevicePolicy=` resource-control setting.
/// See <https://www.freedesktop.org/software/systemd/man/systemd.resource-control.html#DevicePolicy=auto|closed|strict>.
#[derive(Clone, Eq, PartialEq, Debug, serde::Serialize, serde::Deserialize, Default)]
pub enum DevicePolicy {
    /// No restrictions on device access (default).
    #[default]
    Auto,
    /// Only standard pseudo devices (/dev/null, /dev/zero, /dev/full,
    /// /dev/random, /dev/urandom, /dev/tty, /dev/pts/ptmx) plus any
    /// explicitly allowed via DeviceAllow= are accessible.
    Closed,
    /// Only devices explicitly listed in DeviceAllow= are accessible.
    Strict,
}

/// IOSchedulingClass= — sets the I/O scheduling class for executed processes.
/// Matches systemd's `IOSchedulingClass=` exec setting.
/// See <https://www.freedesktop.org/software/systemd/man/systemd.exec.html#IOSchedulingClass=>.
#[derive(
    Clone, Copy, Eq, PartialEq, Hash, Debug, serde::Serialize, serde::Deserialize, Default,
)]
pub enum IOSchedulingClass {
    /// No explicit class set; the kernel default (best-effort) is used.
    #[default]
    None,
    /// Realtime I/O scheduling (class 1). Processes in this class are
    /// always granted I/O access first. Use with caution — can starve
    /// other processes.
    Realtime,
    /// Best-effort I/O scheduling (class 2). The default scheduling class.
    /// I/O bandwidth is distributed based on the I/O priority within this
    /// class.
    BestEffort,
    /// Idle I/O scheduling (class 3). Processes in this class only get
    /// I/O time when no other process needs the disk.
    Idle,
}

/// Controls when a unit is unloaded from memory after it becomes inactive/failed.
/// Matches systemd's `CollectMode=` setting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CollectMode {
    /// Unload the unit when it becomes inactive (default).
    #[default]
    Inactive,
    /// Unload the unit when it becomes inactive or failed.
    InactiveOrFailed,
}

/// Job mode for OnFailure= units.
/// Controls how the triggered failure units are enqueued.
/// Matches systemd's `OnFailureJobMode=` setting.
/// See <https://www.freedesktop.org/software/systemd/man/systemd.unit.html#OnFailureJobMode=>.
#[derive(Clone, Eq, PartialEq, Debug, serde::Serialize, serde::Deserialize, Default)]
pub enum OnFailureJobMode {
    /// Replace existing conflicting jobs (default).
    #[default]
    Replace,
    /// Fail if there are conflicting jobs already queued.
    Fail,
    /// Like `Replace`, but also cancel jobs in units that conflict.
    ReplaceIrreversibly,
    /// Start the unit and stop all other units.
    Isolate,
    /// Cancel all queued jobs and enqueue the new one.
    Flush,
    /// Ignore all unit dependency requirements.
    IgnoreDependencies,
    /// Ignore only `Requires=` dependencies.
    IgnoreRequirements,
}

pub struct ParsedUnitSection {
    pub description: String,
    pub documentation: Vec<String>,

    pub wants: Vec<String>,
    pub requires: Vec<String>,

    /// Like `Requires=`, but the listed units must already be active when this
    /// unit is started. If they are not active, the start is immediately failed
    /// (they are NOT pulled in). Matches systemd's `Requisite=` setting.
    /// Parsed and stored; runtime enforcement adds them to requires list.
    pub requisite: Vec<String>,

    /// Like `Wants=`, but continuously re-starts the listed units if they stop.
    /// Matches systemd's `Upholds=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub upholds: Vec<String>,

    pub conflicts: Vec<String>,
    pub before: Vec<String>,
    pub after: Vec<String>,

    /// Units this unit is "part of". When the listed units are stopped or
    /// restarted, this unit is also stopped or restarted.
    /// Matches systemd's `PartOf=` setting.
    pub part_of: Vec<String>,

    /// Units this unit "binds to". Similar to `Requires=`, but additionally
    /// causes this unit to stop when the listed units stop (even cleanly).
    /// Matches systemd's `BindsTo=` setting.
    /// Parsed and stored; stop-propagation not yet enforced at runtime.
    pub binds_to: Vec<String>,

    /// Whether to add implicit default dependencies (e.g. on sysinit.target / shutdown.target).
    /// Defaults to true, matching systemd behavior.
    pub default_dependencies: bool,

    /// If true, this unit will not be stopped when isolating to another target.
    /// Defaults to false, matching systemd's `IgnoreOnIsolate=` setting.
    pub ignore_on_isolate: bool,

    /// Conditions that must all be true for the unit to start.
    /// If any condition fails, the unit is skipped (not an error).
    /// Matches systemd's ConditionPathExists=, ConditionPathIsDirectory=, etc.
    pub conditions: Vec<UnitCondition>,

    /// Assertions that must all be true for the unit to start.
    /// Unlike conditions, if any assertion fails the unit enters a **failed**
    /// state (not silently skipped). Uses the same `UnitCondition` type since
    /// the check logic is identical — only the failure semantics differ.
    /// Matches systemd's AssertPathExists=, AssertPathIsDirectory=, etc.
    pub assertions: Vec<UnitCondition>,

    /// Action to take when the unit finishes successfully.
    /// Matches systemd's `SuccessAction=` setting.
    pub success_action: UnitAction,

    /// Action to take when the unit fails.
    /// Matches systemd's `FailureAction=` setting.
    pub failure_action: UnitAction,

    /// Exit status to report when `SuccessAction=` triggers.
    /// Only meaningful when SuccessAction= is set to something other than `none`.
    /// Matches systemd's `SuccessActionExitStatus=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub success_action_exit_status: Option<u8>,

    /// Exit status to report when `FailureAction=` triggers.
    /// Only meaningful when FailureAction= is set to something other than `none`.
    /// Matches systemd's `FailureActionExitStatus=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub failure_action_exit_status: Option<u8>,

    /// Absolute paths that this unit requires mount points for.
    /// Automatically adds `Requires=` and `After=` dependencies on the
    /// corresponding `.mount` units for each path prefix.
    /// Matches systemd's `RequiresMountsFor=` setting.
    pub requires_mounts_for: Vec<String>,

    /// If true, this unit is stopped when no other active unit requires or wants it.
    /// Defaults to false, matching systemd's `StopWhenUnneeded=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub stop_when_unneeded: bool,

    /// If true, this unit may be used with `systemctl isolate`.
    /// Defaults to false, matching systemd's `AllowIsolate=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub allow_isolate: bool,

    /// Timeout before a job for this unit is cancelled.
    /// Matches systemd's `JobTimeoutSec=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub job_timeout_sec: Option<Timeout>,

    /// Action to take when a job for this unit times out.
    /// Matches systemd's `JobTimeoutAction=` setting.
    /// Uses the same action values as `SuccessAction=`/`FailureAction=`.
    /// Parsed and stored; no runtime enforcement yet.
    pub job_timeout_action: UnitAction,

    /// Timeout for the running phase of a job (after it has been started).
    /// Matches systemd's `JobRunningTimeoutSec=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub job_running_timeout_sec: Option<Timeout>,

    /// Reboot argument string passed to reboot(2) if JobTimeoutAction=reboot-*.
    /// Matches systemd's `JobTimeoutRebootArgument=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub job_timeout_reboot_argument: Option<String>,

    /// If true, this unit may not be started manually (e.g. via `systemctl start`).
    /// It can only be started as a dependency of another unit.
    /// Defaults to false, matching systemd's `RefuseManualStart=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub refuse_manual_start: bool,

    /// If true, this unit may not be stopped manually (e.g. via `systemctl stop`).
    /// It can only be stopped as a dependency of another unit.
    /// Defaults to false, matching systemd's `RefuseManualStop=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub refuse_manual_stop: bool,

    /// Units to activate when this unit completes successfully.
    /// Matches systemd's `OnSuccess=` setting.
    /// Parsed and stored; no runtime triggering enforcement yet.
    pub on_success: Vec<String>,

    /// Job mode for enqueuing OnSuccess= units.
    /// Defaults to `Replace`, matching systemd's default.
    /// Matches systemd's `OnSuccessJobMode=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub on_success_job_mode: OnFailureJobMode,

    /// Units to activate when this unit enters the "failed" state.
    /// Matches systemd's `OnFailure=` setting.
    /// Parsed and stored; no runtime triggering enforcement yet.
    pub on_failure: Vec<String>,

    /// Job mode for enqueuing OnFailure= units.
    /// Defaults to `Replace`, matching systemd's default.
    /// Matches systemd's `OnFailureJobMode=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub on_failure_job_mode: OnFailureJobMode,

    /// Units to propagate reload requests to.
    /// When this unit is reloaded, the listed units are also reloaded.
    /// Matches systemd's `PropagatesReloadTo=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub propagates_reload_to: Vec<String>,

    /// Units whose reload requests are propagated to this unit.
    /// When the listed units are reloaded, this unit is also reloaded.
    /// Matches systemd's `ReloadPropagatedFrom=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub reload_propagated_from: Vec<String>,

    /// Units to propagate stop requests to.
    /// When this unit is stopped, the listed units are also stopped.
    /// Matches systemd's `PropagatesStopTo=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub propagates_stop_to: Vec<String>,

    /// Units that share the same mount namespace as this unit.
    /// Matches systemd's `JoinsNamespaceOf=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub joins_namespace_of: Vec<String>,

    /// Time interval for rate limiting unit starts.
    /// If the unit is started more than `start_limit_burst` times within this interval,
    /// further starts are denied.
    /// Matches systemd's `StartLimitIntervalSec=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub start_limit_interval_sec: Option<Timeout>,

    /// Number of starts allowed within the `start_limit_interval_sec` interval.
    /// Defaults to `None` (use manager default, typically 5).
    /// Matches systemd's `StartLimitBurst=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub start_limit_burst: Option<u32>,

    /// Action to take when the start rate limit is hit.
    /// Matches systemd's `StartLimitAction=` setting.
    /// Uses the same action values as `SuccessAction=`/`FailureAction=`.
    /// Parsed and stored; no runtime enforcement yet.
    pub start_limit_action: UnitAction,

    /// Controls when a unit is unloaded from memory.
    /// Matches systemd's `CollectMode=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub collect_mode: CollectMode,

    /// Path to the source configuration file that generated this unit.
    /// Matches systemd's `SourcePath=` setting.
    /// Parsed and stored; informational only.
    pub source_path: Option<String>,

    /// Reboot argument string passed to reboot(2) when SuccessAction=/FailureAction=
    /// triggers a reboot. Matches systemd's `RebootArgument=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub reboot_argument: Option<String>,
}

impl Default for ParsedUnitSection {
    fn default() -> Self {
        Self {
            description: String::new(),
            documentation: Vec::new(),
            wants: Vec::new(),
            requires: Vec::new(),
            requisite: Vec::new(),
            upholds: Vec::new(),
            conflicts: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            part_of: Vec::new(),
            binds_to: Vec::new(),
            default_dependencies: true,
            ignore_on_isolate: false,
            conditions: Vec::new(),
            assertions: Vec::new(),
            success_action: UnitAction::default(),
            failure_action: UnitAction::default(),
            success_action_exit_status: None,
            failure_action_exit_status: None,
            requires_mounts_for: Vec::new(),
            stop_when_unneeded: false,
            allow_isolate: false,
            job_timeout_sec: None,
            job_timeout_action: UnitAction::default(),
            job_running_timeout_sec: None,
            job_timeout_reboot_argument: None,
            refuse_manual_start: false,
            refuse_manual_stop: false,
            on_success: Vec::new(),
            on_success_job_mode: OnFailureJobMode::default(),
            on_failure: Vec::new(),
            on_failure_job_mode: OnFailureJobMode::default(),
            propagates_reload_to: Vec::new(),
            reload_propagated_from: Vec::new(),
            propagates_stop_to: Vec::new(),
            joins_namespace_of: Vec::new(),
            start_limit_interval_sec: None,
            start_limit_burst: None,
            start_limit_action: UnitAction::default(),
            collect_mode: CollectMode::default(),
            source_path: None,
            reboot_argument: None,
        }
    }
}
#[derive(Clone)]
pub struct ParsedSingleSocketConfig {
    pub kind: crate::sockets::SocketKind,
    pub specialized: crate::sockets::SpecializedSocketConfig,
}

impl std::fmt::Debug for ParsedSingleSocketConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(
            f,
            "SocketConfig {{ kind: {:?}, specialized: {:?} }}",
            self.kind, self.specialized
        )?;
        Ok(())
    }
}

pub struct ParsedSocketSection {
    pub sockets: Vec<ParsedSingleSocketConfig>,
    pub filedesc_name: Option<String>,
    pub services: Vec<String>,

    /// Accept= — whether the socket uses Accept mode (inetd-style).
    /// If true, for each incoming connection a new service instance is
    /// spawned. Defaults to false. Parsed and stored; inetd-style
    /// activation is not yet supported at runtime. See systemd.socket(5).
    pub accept: bool,

    /// MaxConnections= — the maximum number of simultaneous connections for
    /// Accept=yes sockets. Defaults to 64. Parsed and stored; no runtime
    /// enforcement yet. See systemd.socket(5).
    pub max_connections: u64,

    /// MaxConnectionsPerSource= — the maximum number of simultaneous
    /// connections for a service per source IP address. Defaults to the value
    /// of MaxConnections=. Parsed and stored; no runtime enforcement yet.
    /// See systemd.socket(5).
    pub max_connections_per_source: u64,

    /// SocketMode= — the file system access mode for the socket node
    /// (AF_UNIX sockets and FIFOs). Takes an octal access mode, e.g. 0666.
    /// Defaults to 0666. Parsed and stored; no runtime enforcement yet.
    /// See systemd.socket(5).
    pub socket_mode: Option<u32>,

    /// DirectoryMode= — the file system access mode for directories created
    /// for the socket (if needed). Takes an octal access mode, e.g. 0755.
    /// Defaults to 0755. Parsed and stored; no runtime enforcement yet.
    /// See systemd.socket(5).
    pub directory_mode: Option<u32>,

    /// PassCredentials= — whether to enable SO_PASSCRED on the socket, so
    /// that the receiving process can obtain peer credentials via an
    /// ancillary message. Defaults to false. Parsed and stored; no runtime
    /// enforcement yet. See systemd.socket(5).
    pub pass_credentials: bool,

    /// PassSecurity= — whether to enable SO_PASSSEC on the socket, so
    /// that AF_UNIX sockets receive the security context of the sending
    /// process in an ancillary message. Defaults to false. Parsed and
    /// stored; no runtime enforcement yet. See systemd.socket(5).
    pub pass_security: bool,

    /// AcceptFileDescriptors= — whether to enable SO_PASSRIGHTS on the
    /// socket, which when disabled prohibits the peer from sending
    /// SCM_RIGHTS ancillary messages (file descriptors) via AF_UNIX
    /// sockets. Defaults to true. Parsed and stored; no runtime
    /// enforcement yet. See systemd.socket(5).
    pub accept_file_descriptors: bool,

    /// RemoveOnStop= — whether to remove file nodes (AF_UNIX sockets,
    /// FIFOs, message queues, and symlinks configured with Symlinks=)
    /// created by this socket unit when it is stopped. Defaults to false.
    /// Parsed and stored; no runtime enforcement yet. See systemd.socket(5).
    pub remove_on_stop: bool,

    /// ReceiveBuffer= — the receive buffer size (SO_RCVBUF) in bytes for the
    /// socket. Takes an integer value. Parsed and stored; no runtime
    /// enforcement yet. See systemd.socket(5).
    pub receive_buffer: Option<u64>,

    /// SendBuffer= — the send buffer size (SO_SNDBUF) in bytes for the
    /// socket. Takes an integer value. Parsed and stored; no runtime
    /// enforcement yet. See systemd.socket(5).
    pub send_buffer: Option<u64>,

    /// Symlinks= — a list of file system paths to create as symlinks to
    /// the AF_UNIX socket path or FIFO path of this socket unit. Multiple
    /// paths may be specified space-separated; the directive may appear
    /// multiple times to extend the list. An empty value resets the list.
    /// Defaults to an empty list. Parsed and stored; no runtime enforcement
    /// yet. See systemd.socket(5).
    pub symlinks: Vec<String>,

    /// Timestamping= — controls the SO_TIMESTAMP or SO_TIMESTAMPNS socket
    /// options, enabling timestamping metadata on ingress network traffic.
    /// Takes one of "off", "us"/"usec"/"μs", or "ns"/"nsec".
    /// Defaults to Off. Parsed and stored; no runtime enforcement yet.
    /// See systemd.socket(5).
    pub timestamping: Timestamping,

    /// DeferTrigger= — controls whether to defer triggering the associated
    /// service when a connection comes in. Takes a boolean or "patient".
    /// Defaults to No. Parsed and stored; no runtime enforcement yet.
    /// See systemd.socket(5).
    pub defer_trigger: DeferTrigger,

    /// Writable= — whether to open the FIFO or special file for writing
    /// as well (i.e. O_RDWR rather than O_RDONLY). Defaults to false.
    /// Parsed and stored; no runtime enforcement yet. See systemd.socket(5).
    pub writable: bool,

    /// Backlog= — the maximum length of the queue of pending connections
    /// for stream sockets (the second argument to `listen()`). Takes an
    /// unsigned integer. Defaults to 128 (SOMAXCONN). See systemd.socket(5).
    pub backlog: Option<u32>,

    /// BindIPv6Only= — controls the IPV6_V6ONLY socket option. Takes
    /// "default", "both", or "ipv6-only". "default" leaves it to the
    /// kernel (usually controlled by sysctl net.ipv6.bindv6only).
    /// "both" clears IPV6_V6ONLY so the socket accepts both IPv4 and IPv6.
    /// "ipv6-only" sets IPV6_V6ONLY so only IPv6 is accepted.
    /// Defaults to "default". See systemd.socket(5).
    pub bind_ipv6_only: BindIPv6Only,

    /// BindToDevice= — bind the socket to a specific network interface
    /// (SO_BINDTODEVICE). Takes a network interface name. An empty string
    /// resets the binding. See systemd.socket(5).
    pub bind_to_device: Option<String>,

    /// SocketUser= — the UNIX user that owns the socket file node
    /// (AF_UNIX sockets and FIFOs). Takes a UNIX username or numeric UID.
    /// Defaults to root. See systemd.socket(5).
    pub socket_user: Option<String>,

    /// SocketGroup= — the UNIX group that owns the socket file node
    /// (AF_UNIX sockets and FIFOs). Takes a UNIX group name or numeric GID.
    /// Defaults to root. See systemd.socket(5).
    pub socket_group: Option<String>,

    /// FreeBind= — whether to set the IP_FREEBIND/IPV6_FREEBIND socket
    /// option, which allows binding to addresses not yet assigned to an
    /// interface. Defaults to false. See systemd.socket(5).
    pub free_bind: bool,

    /// Transparent= — whether to set the IP_TRANSPARENT socket option,
    /// which allows binding to non-local addresses and transparent proxying.
    /// Defaults to false. See systemd.socket(5).
    pub transparent: bool,

    /// Broadcast= — whether to set the SO_BROADCAST socket option, which
    /// allows sending to broadcast addresses on datagram sockets.
    /// Defaults to false. See systemd.socket(5).
    pub broadcast: bool,

    /// ReusePort= — whether to set the SO_REUSEPORT socket option, which
    /// allows multiple sockets to bind to the same port. Each accepting
    /// socket will get connections distributed by the kernel.
    /// Defaults to false. See systemd.socket(5).
    pub reuse_port: bool,

    /// KeepAlive= — whether to enable SO_KEEPALIVE on TCP sockets.
    /// Defaults to false. See systemd.socket(5).
    pub keep_alive: bool,

    /// KeepAliveTimeSec= — the idle time before TCP starts sending
    /// keepalive probes (TCP_KEEPIDLE). Takes a timespan value.
    /// Only meaningful when KeepAlive=yes. See systemd.socket(5).
    pub keep_alive_time_sec: Option<u64>,

    /// KeepAliveIntervalSec= — the interval between TCP keepalive probes
    /// (TCP_KEEPINTVL). Takes a timespan value. Only meaningful when
    /// KeepAlive=yes. See systemd.socket(5).
    pub keep_alive_interval_sec: Option<u64>,

    /// KeepAliveProbes= — the number of unacknowledged TCP keepalive
    /// probes before the connection is considered dead (TCP_KEEPCNT).
    /// Takes an unsigned integer. Only meaningful when KeepAlive=yes.
    /// See systemd.socket(5).
    pub keep_alive_probes: Option<u32>,

    /// NoDelay= — whether to enable TCP_NODELAY on TCP sockets, disabling
    /// Nagle's algorithm. Defaults to false. See systemd.socket(5).
    pub no_delay: bool,

    /// Priority= — the socket priority (SO_PRIORITY). Takes an integer.
    /// See systemd.socket(5).
    pub priority: Option<i32>,

    /// Mark= — the firewall mark (SO_MARK) to set on packets sent through
    /// this socket. Takes an unsigned integer. See systemd.socket(5).
    pub mark: Option<u32>,

    /// IPTOS= — the IP Type-Of-Service byte (IP_TOS) for packets sent on
    /// this socket. Takes an integer or one of "low-delay", "throughput",
    /// "reliability", "low-cost". See systemd.socket(5).
    pub ip_tos: Option<i32>,

    /// IPTTL= — the IP Time-To-Live (IP_TTL) for packets sent on this
    /// socket. Takes an integer 1–255. See systemd.socket(5).
    pub ip_ttl: Option<u32>,

    /// PipeSize= — the pipe buffer size (F_SETPIPE_SZ) in bytes for
    /// FIFOs. Takes a byte size value. See systemd.socket(5).
    pub pipe_size: Option<u64>,

    /// FlushPending= — whether to flush the socket of pending data on
    /// the first accepted connection. Takes a boolean. Defaults to false.
    /// See systemd.socket(5).
    pub flush_pending: bool,

    /// TriggerLimitIntervalSec= — rate-limiting: the time interval within
    /// which TriggerLimitBurst= activations are permitted before the socket
    /// unit is put into a failure state. Takes a timespan. Defaults to 2s.
    /// See systemd.socket(5).
    pub trigger_limit_interval_sec: Option<u64>,

    /// TriggerLimitBurst= — rate-limiting: the maximum number of socket
    /// activations permitted within TriggerLimitIntervalSec= before the
    /// socket unit enters a failure state. Takes an unsigned integer.
    /// Defaults to 200. See systemd.socket(5).
    pub trigger_limit_burst: Option<u32>,

    /// SocketProtocol= — the socket protocol (third argument to socket(2)).
    /// Takes "udplite" or "sctp". Only useful with ListenStream= or
    /// ListenDatagram= on IP sockets. See systemd.socket(5).
    pub socket_protocol: Option<String>,

    /// SELinuxContextFromNet= — whether to set the SELinux context of
    /// accepted connections from the network peer. Takes a boolean.
    /// Defaults to false. See systemd.socket(5).
    pub selinux_context_from_net: bool,

    /// SmackLabel= — the SMACK security label for the socket file node.
    /// See systemd.socket(5).
    pub smack_label: Option<String>,

    /// SmackLabelIPIn= — the SMACK security label for incoming IP packets.
    /// See systemd.socket(5).
    pub smack_label_ipin: Option<String>,

    /// SmackLabelIPOut= — the SMACK security label for outgoing IP packets.
    /// See systemd.socket(5).
    pub smack_label_ipout: Option<String>,

    /// PassPacketInfo= — whether to enable IP_PKTINFO (IPv4) or
    /// IPV6_RECVPKTINFO (IPv6) on the socket, so that the receiving
    /// process can determine the destination address and interface of
    /// incoming packets via ancillary messages. Defaults to false.
    /// See systemd.socket(5).
    pub pass_packet_info: bool,

    /// TCPCongestion= — the TCP congestion control algorithm to set on
    /// the socket (TCP_CONGESTION). Takes a string such as "cubic",
    /// "reno", "bbr", etc. If not set, the kernel default is used.
    /// See systemd.socket(5).
    pub tcp_congestion: Option<String>,

    /// ExecStartPre= — commands to execute before listening on the socket(s).
    /// Multiple lines are allowed and executed in order. Supports the same
    /// command prefixes as ExecStart= in service units ("-" for ignore-failure,
    /// "@" for argv[0] override). See systemd.socket(5).
    pub exec_start_pre: Vec<Commandline>,

    /// ExecStartPost= — commands to execute after listening on the socket(s)
    /// has been set up. See systemd.socket(5).
    pub exec_start_post: Vec<Commandline>,

    /// ExecStopPre= — commands to execute before closing the socket(s).
    /// See systemd.socket(5).
    pub exec_stop_pre: Vec<Commandline>,

    /// ExecStopPost= — commands to execute after closing the socket(s).
    /// See systemd.socket(5).
    pub exec_stop_post: Vec<Commandline>,

    /// TimeoutSec= — timeout for ExecStartPre=, ExecStartPost=,
    /// ExecStopPre=, and ExecStopPost= commands. If any of these commands
    /// does not finish within the configured time, the socket unit is
    /// considered failed. Takes a timespan value or "infinity".
    /// Defaults to DefaultTimeoutStartSec= from manager configuration.
    /// See systemd.socket(5).
    pub timeout_sec: Option<Timeout>,

    /// PassFileDescriptorsToExec= — whether to pass the socket file
    /// descriptors to ExecStartPost=, ExecStopPre=, and ExecStopPost=
    /// commands via the sd_listen_fds(3) protocol. Takes a boolean.
    /// Defaults to false. See systemd.socket(5). (systemd 257+)
    pub pass_file_descriptors_to_exec: bool,

    /// PollLimitIntervalSec= — rate-limiting interval for poll/select
    /// wakeups. Defaults to 0 (disabled). See systemd.socket(5).
    pub poll_limit_interval_sec: Option<u64>,

    /// PollLimitBurst= — maximum poll/select wakeups within the interval.
    /// Defaults to 0 (disabled). See systemd.socket(5).
    pub poll_limit_burst: Option<u32>,

    pub exec_section: ParsedExecSection,
}

/// BindIPv6Only= mode for IPv6 sockets. See systemd.socket(5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindIPv6Only {
    /// Leave it to the kernel default (sysctl net.ipv6.bindv6only).
    Default,
    /// Clear IPV6_V6ONLY — accept both IPv4-mapped and IPv6 connections.
    Both,
    /// Set IPV6_V6ONLY — accept only IPv6 connections.
    Ipv6Only,
}
pub struct ParsedServiceSection {
    pub restart: ServiceRestart,
    pub restart_sec: Option<Timeout>,
    pub kill_mode: KillMode,
    pub delegate: Delegate,
    pub tasks_max: Option<TasksMax>,
    pub limit_nofile: Option<ResourceLimit>,
    pub accept: bool,
    pub notifyaccess: NotifyKind,
    pub exec: Vec<Commandline>,
    pub reload: Vec<Commandline>,
    pub stop: Vec<Commandline>,
    pub stoppost: Vec<Commandline>,
    pub startpre: Vec<Commandline>,
    pub startpost: Vec<Commandline>,
    pub srcv_type: ServiceType,
    pub starttimeout: Option<Timeout>,
    pub stoptimeout: Option<Timeout>,
    pub generaltimeout: Option<Timeout>,

    pub dbus_name: Option<String>,
    /// PIDFile= — path to a file that contains the PID of the main daemon
    /// process after a Type=forking service has started.
    pub pid_file: Option<PathBuf>,

    pub sockets: Vec<String>,

    /// Slice= — the slice unit to place this service in for resource management
    pub slice: Option<String>,

    /// RemainAfterExit= — whether the service is considered active even after
    /// the main process exits. Defaults to false. Commonly used with Type=oneshot.
    pub remain_after_exit: bool,

    /// SuccessExitStatus= — additional exit codes and signals that are
    /// considered a successful (clean) service termination.
    pub success_exit_status: crate::units::SuccessExitStatus,

    /// RestartForceExitStatus= — a list of exit status definitions (exit
    /// codes and/or signal names) that will force automatic service restart
    /// regardless of the configured Restart= setting. The format is the
    /// same as SuccessExitStatus=. See systemd.service(5).
    pub restart_force_exit_status: crate::units::SuccessExitStatus,

    /// SendSIGHUP= — if true, send SIGHUP to remaining processes immediately
    /// after the stop signal (e.g. SIGTERM). This is useful for shell-like
    /// services that need to be notified their connection has been severed.
    /// Defaults to false. See systemd.kill(5).
    pub send_sighup: bool,

    /// MemoryPressureWatch= — configures whether to watch for memory pressure
    /// events via PSI. Parsed and stored; no runtime enforcement.
    /// See systemd.resource-control(5).
    pub memory_pressure_watch: MemoryPressureWatch,

    /// ReloadSignal= — configures the UNIX process signal to send to the
    /// service's main process when asked to reload. Defaults to SIGHUP.
    /// Only effective with Type=notify-reload. Stored as a raw signal
    /// number to support realtime signals (RTMIN+N). Parsed and stored;
    /// not yet used at runtime. See systemd.service(5).
    pub reload_signal: Option<i32>,

    /// KillSignal= — specifies which signal to use when stopping the service.
    /// Defaults to SIGTERM. Takes a signal name such as SIGTERM, SIGINT,
    /// SIGKILL, RTMIN+N, or a numeric signal number. Stored as a raw signal
    /// number to support realtime signals. Parsed and stored; not yet used
    /// at runtime. See systemd.kill(5).
    pub kill_signal: Option<i32>,

    /// DelegateSubgroup= — place unit processes in the specified subgroup of
    /// the unit's control group. Only effective when Delegate= is enabled.
    /// Parsed and stored; not yet used at runtime. See systemd.resource-control(5).
    pub delegate_subgroup: Option<String>,

    /// KeyringMode= — controls how the kernel session keyring is set up for
    /// the service. Defaults to `private` for system services and `inherit`
    /// for non-service units / user services. Parsed and stored; not yet
    /// enforced at runtime. See systemd.exec(5).
    pub keyring_mode: KeyringMode,

    /// DeviceAllow= — controls access to specific device nodes for the unit.
    /// Each entry is a device node path (or class like "char-*", "block-*")
    /// optionally followed by access characters (r=read, w=write, m=mknod).
    /// Multiple directives accumulate; an empty assignment resets the list.
    /// Parsed and stored; no runtime enforcement yet (requires cgroup device
    /// controller). See systemd.resource-control(5).
    pub device_allow: Vec<String>,

    /// DevicePolicy= — controls the policy for device access for the unit.
    /// Takes "auto" (default, no restrictions), "closed" (only standard
    /// pseudo devices plus DeviceAllow= entries), or "strict" (only
    /// DeviceAllow= entries). Parsed and stored; no runtime enforcement
    /// yet (requires cgroup device controller). See systemd.resource-control(5).
    pub device_policy: DevicePolicy,

    /// WatchdogSec= — configures the watchdog timeout for the service. The
    /// service must send a "WATCHDOG=1" notification via sd_notify() at least
    /// once within this interval, or it will be considered failed. A value of
    /// 0 or empty disables the watchdog. Parsed and stored; no runtime
    /// enforcement yet. See systemd.service(5).
    pub watchdog_sec: Option<Timeout>,

    /// IPAddressAllow= — a list of IP address prefixes (CIDR notation) or
    /// special keywords (`any`, `localhost`, `link-local`, `multicast`) that
    /// are allowed for IP communication by this unit. Multiple directives
    /// accumulate; an empty assignment resets the list. Parsed and stored;
    /// no runtime enforcement yet (requires eBPF/cgroup support).
    /// See systemd.resource-control(5).
    pub ip_address_allow: Vec<String>,

    /// IPAddressDeny= — a list of IP address prefixes (CIDR notation) or
    /// special keywords (`any`, `localhost`, `link-local`, `multicast`) that
    /// are denied for IP communication by this unit. Multiple directives
    /// accumulate; an empty assignment resets the list. Parsed and stored;
    /// no runtime enforcement yet (requires eBPF/cgroup support).
    /// See systemd.resource-control(5).
    pub ip_address_deny: Vec<String>,

    /// FileDescriptorStoreMax= — the maximum number of file descriptors that
    /// may be stored in the service manager for this service using
    /// sd_pid_notify_with_fds()'s FDSTORE=1 messages. Defaults to 0
    /// (file descriptor store disabled). Parsed and stored; no runtime
    /// enforcement yet. See systemd.service(5).
    pub file_descriptor_store_max: u64,

    /// FileDescriptorStorePreserve= — controls whether file descriptors
    /// stored in the service manager (via FDSTORE=1 sd_notify messages) are
    /// preserved across service restarts or stops. Takes `no` (default,
    /// discard on stop), `yes` (preserve as long as the unit exists), or
    /// `restart` (preserve across restarts, discard on full stop). Parsed
    /// and stored; no runtime enforcement yet. See systemd.service(5).
    pub file_descriptor_store_preserve: FileDescriptorStorePreserve,

    /// MemoryMin= — minimum memory guarantee for the unit's cgroup. The
    /// memory controller will try to protect at least this much memory from
    /// reclaim. Accepts a byte value with optional K/M/G/T/P/E suffix
    /// (base 1024), a percentage, or "infinity". Parsed and stored; applied
    /// to cgroup memory.min at runtime. See systemd.resource-control(5).
    pub memory_min: Option<MemoryLimit>,

    /// MemoryLow= — low memory boundary for the unit's cgroup. Below this
    /// threshold the kernel memory reclaimer will avoid reclaiming memory
    /// from the unit. Accepts a byte value with optional K/M/G/T/P/E suffix
    /// (base 1024), a percentage, or "infinity". Parsed and stored; applied
    /// to cgroup memory.low at runtime. See systemd.resource-control(5).
    pub memory_low: Option<MemoryLimit>,

    /// MemoryHigh= — throttling memory boundary for the unit's cgroup. If
    /// memory usage goes above this threshold, processes are heavily throttled
    /// and put under pressure. Accepts a byte value with optional K/M/G/T/P/E
    /// suffix (base 1024), a percentage, or "infinity". Applied to cgroup
    /// memory.high at runtime. See systemd.resource-control(5).
    pub memory_high: Option<MemoryLimit>,

    /// MemoryMax= — hard memory limit for the unit's cgroup. If memory usage
    /// exceeds this limit, the OOM killer is invoked. Accepts a byte value with
    /// optional K/M/G/T/P/E suffix (base 1024), a percentage, or "infinity".
    /// Applied to cgroup memory.max at runtime. See systemd.resource-control(5).
    pub memory_max: Option<MemoryLimit>,

    /// MemorySwapMax= — hard swap limit for the unit's cgroup. Accepts a byte
    /// value with optional K/M/G/T/P/E suffix (base 1024), a percentage, or
    /// "infinity". Applied to cgroup memory.swap.max at runtime.
    /// See systemd.resource-control(5).
    pub memory_swap_max: Option<MemoryLimit>,

    /// CPUWeight= — CPU weight for the unit's cgroup (1–10000, default 100).
    /// Applied to cgroup cpu.weight at runtime. See systemd.resource-control(5).
    pub cpu_weight: Option<CpuWeight>,

    /// StartupCPUWeight= — CPU weight during system startup (1–10000).
    /// Parsed and stored; no runtime enforcement yet (falls back to CPUWeight=).
    /// See systemd.resource-control(5).
    pub startup_cpu_weight: Option<CpuWeight>,

    /// CPUQuota= — CPU time quota as a percentage (e.g. "20%", "200%").
    /// Applied to cgroup cpu.max at runtime. See systemd.resource-control(5).
    pub cpu_quota: Option<CpuQuota>,

    /// IOWeight= — I/O weight for the unit's cgroup (1–10000, default 100).
    /// Applied to cgroup io.weight at runtime. See systemd.resource-control(5).
    pub io_weight: Option<IoWeight>,

    /// StartupIOWeight= — I/O weight during system startup (1–10000).
    /// Parsed and stored; no runtime enforcement yet (falls back to IOWeight=).
    /// See systemd.resource-control(5).
    pub startup_io_weight: Option<IoWeight>,

    /// IODeviceWeight= — per-device I/O weight (format: "/dev/path WEIGHT").
    /// Applied to cgroup io.weight at runtime. See systemd.resource-control(5).
    pub io_device_weight: Vec<IoDeviceLimit>,

    /// IOReadBandwidthMax= — per-device read bandwidth limit (format:
    /// "/dev/path BYTES"). Applied to cgroup io.max at runtime.
    /// See systemd.resource-control(5).
    pub io_read_bandwidth_max: Vec<IoDeviceLimit>,

    /// IOWriteBandwidthMax= — per-device write bandwidth limit (format:
    /// "/dev/path BYTES"). Applied to cgroup io.max at runtime.
    /// See systemd.resource-control(5).
    pub io_write_bandwidth_max: Vec<IoDeviceLimit>,

    /// IOReadIOPSMax= — per-device read IOPS limit (format: "/dev/path IOPS").
    /// Applied to cgroup io.max at runtime. See systemd.resource-control(5).
    pub io_read_iops_max: Vec<IoDeviceLimit>,

    /// IOWriteIOPSMax= — per-device write IOPS limit (format: "/dev/path IOPS").
    /// Applied to cgroup io.max at runtime. See systemd.resource-control(5).
    pub io_write_iops_max: Vec<IoDeviceLimit>,

    /// CPUAccounting= — enable CPU accounting for the unit's cgroup.
    /// When true, the cpu controller is enabled. Defaults to the system default.
    /// See systemd.resource-control(5).
    pub cpu_accounting: Option<bool>,

    /// MemoryAccounting= — enable memory accounting for the unit's cgroup.
    /// When true, the memory controller is enabled. Defaults to the system default.
    /// See systemd.resource-control(5).
    pub memory_accounting: Option<bool>,

    /// IOAccounting= — enable I/O accounting for the unit's cgroup.
    /// When true, the io controller is enabled. Defaults to the system default.
    /// See systemd.resource-control(5).
    pub io_accounting: Option<bool>,

    /// TasksAccounting= — enable task counting for the unit's cgroup.
    /// When true, the pids controller is enabled. Defaults to the system default.
    /// See systemd.resource-control(5).
    pub tasks_accounting: Option<bool>,

    /// RuntimeMaxSec= — configures a maximum time for the service to run.
    /// If the service has been active for longer than the specified time it
    /// is terminated and put into a failure state. When set to `infinity`
    /// (the default) or not set, no time limit is configured. A value of 0
    /// also means no limit. Parsed and stored; no runtime enforcement yet.
    /// See systemd.service(5).
    pub runtime_max_sec: Option<Timeout>,

    /// CoredumpReceive= — boolean, defaults to false. When enabled, the
    /// service manager sets up a coredump socket so the service can receive
    /// coredumps from systemd-coredump. Parsed and stored; no runtime
    /// enforcement yet. See systemd.service(5).
    pub coredump_receive: bool,

    /// SendSIGKILL= — if true (the default), send SIGKILL to remaining
    /// processes after the stop timeout if they haven't terminated yet.
    /// If false, leave processes running after the timeout.
    /// See systemd.kill(5).
    pub send_sigkill: bool,

    /// RestartKillSignal= — specifies which signal to use when restarting
    /// a service (i.e. stop-before-restart). Takes a signal name or number.
    /// When not set, KillSignal= is used. See systemd.kill(5).
    pub restart_kill_signal: Option<i32>,

    /// FinalKillSignal= — specifies the signal to send as the very last
    /// step when terminating a service, after SIGKILL has been sent and the
    /// processes still haven't terminated. Defaults to SIGKILL.
    /// Takes a signal name or number. See systemd.kill(5).
    pub final_kill_signal: Option<i32>,

    /// WatchdogSignal= — specifies the signal to use to kill a service
    /// when the watchdog timeout expires. Defaults to SIGABRT.
    /// Takes a signal name or number. See systemd.kill(5).
    pub watchdog_signal: Option<i32>,

    /// ExitType= — configures when a service is considered exited/dead.
    /// `main` (default): the service is dead when the main process exits.
    /// `cgroup`: the service is dead only when the last process in its
    /// cgroup exits. See systemd.service(5).
    pub exit_type: ExitType,

    /// OOMPolicy= — configures the Out-Of-Memory killer policy for the
    /// service. Takes `continue`, `stop` (default), or `kill`.
    /// See systemd.service(5).
    pub oom_policy: OOMPolicy,

    /// TimeoutAbortSec= — configures the timeout for aborting the service
    /// (e.g. when the watchdog triggers). If not set, TimeoutStopSec= is
    /// used. See systemd.service(5).
    pub timeout_abort_sec: Option<Timeout>,

    /// TimeoutCleanSec= — configures the timeout for the clean-up phase
    /// of ExecStopPost= commands when cleaning resources on explicit request.
    /// If not set, TimeoutStopSec= is used. See systemd.service(5).
    pub timeout_clean_sec: Option<Timeout>,

    /// RestartPreventExitStatus= — a list of exit status definitions (exit
    /// codes and/or signal names) that will prevent automatic service restart
    /// regardless of the configured Restart= setting. The format is the
    /// same as SuccessExitStatus=. See systemd.service(5).
    pub restart_prevent_exit_status: crate::units::SuccessExitStatus,

    /// RestartMode= — configures how a service is restarted. Takes `direct`
    /// (default) or `normal`. See systemd.service(5).
    pub restart_mode: RestartMode,

    /// RestartSteps= — configures the number of steps in the graduated
    /// restart delay ramp-up from RestartSec= to RestartMaxDelaySec=.
    /// A value of 0 (default) disables the graduated delay feature.
    /// See systemd.service(5).
    pub restart_steps: u32,

    /// RestartMaxDelaySec= — configures the longest restart delay when
    /// graduated restart delay is in use (RestartSteps= > 0). If not set
    /// or set to `infinity`, no upper bound is imposed.
    /// See systemd.service(5).
    pub restart_max_delay_sec: Option<Timeout>,

    /// ExecCondition= — commands to execute before ExecStartPre=. If any
    /// command exits with a non-zero status (and without the `-` prefix),
    /// the rest of the commands are skipped and the unit is not started,
    /// but this is not considered a failure. Exit code 254 causes the
    /// condition to be treated as failed (unit skipped).
    /// See systemd.service(5).
    pub exec_condition: Vec<Commandline>,

    // --- New service directives (systemd.service(5)) ---
    /// GuessMainPID= — if true (default), the service manager will try to
    /// guess the main PID of a forking service if PIDFile= is not set.
    /// See systemd.service(5).
    pub guess_main_pid: bool,

    /// TimeoutStartFailureMode= — configures the action to take when a
    /// start timeout is reached. Takes "terminate" (default) or "abort".
    /// See systemd.service(5).
    pub timeout_start_failure_mode: TimeoutFailureMode,

    /// TimeoutStopFailureMode= — configures the action to take when a
    /// stop timeout is reached. Takes "terminate" (default) or "abort".
    /// See systemd.service(5).
    pub timeout_stop_failure_mode: TimeoutFailureMode,

    /// RuntimeRandomizedExtraSec= — an additional random time added on
    /// top of RuntimeMaxSec=. Defaults to 0 (disabled). Parsed and stored;
    /// no runtime enforcement yet. See systemd.service(5).
    pub runtime_randomized_extra_sec: Option<Timeout>,

    /// RootDirectoryStartOnly= — if true, the root directory (as configured
    /// with RootDirectory=) is only applied to ExecStart= and not to the
    /// other Exec*= settings. Defaults to false. See systemd.service(5).
    pub root_directory_start_only: bool,

    /// NonBlocking= — if true, all file descriptors passed via socket
    /// activation are set to non-blocking mode (O_NONBLOCK). Defaults to
    /// false. See systemd.service(5).
    pub non_blocking: bool,

    /// USBFunctionDescriptors= — path to a file containing USB FunctionFS
    /// descriptors for the USB gadget. See systemd.service(5).
    pub usb_function_descriptors: Option<std::path::PathBuf>,

    /// USBFunctionStrings= — path to a file containing USB FunctionFS
    /// strings for the USB gadget. See systemd.service(5).
    pub usb_function_strings: Option<std::path::PathBuf>,

    /// OpenFile= — a list of file paths to open and pass as file descriptors
    /// to the service. Format: "path[:fdname[:options]]". Multiple directives
    /// accumulate; an empty assignment resets the list. See systemd.service(5).
    pub open_file: Vec<String>,

    // --- New resource-control directives (systemd.resource-control(5)) ---
    /// CPUQuotaPeriodSec= — the period for CPU quota enforcement. Defaults
    /// to 100ms. Applied to cgroup cpu.max at runtime.
    /// See systemd.resource-control(5).
    pub cpu_quota_period_sec: Option<Timeout>,

    /// AllowedCPUs= — restricts processes to the specified CPUs (cpuset
    /// controller). Takes a CPU index list with ranges. Applied to
    /// cgroup cpuset.cpus at runtime. See systemd.resource-control(5).
    pub allowed_cpus: Option<String>,

    /// StartupAllowedCPUs= — like AllowedCPUs= but only during startup.
    /// See systemd.resource-control(5).
    pub startup_allowed_cpus: Option<String>,

    /// AllowedMemoryNodes= — restricts processes to the specified NUMA
    /// memory nodes. Takes a node index list with ranges. Applied to
    /// cgroup cpuset.mems at runtime. See systemd.resource-control(5).
    pub allowed_memory_nodes: Option<String>,

    /// StartupAllowedMemoryNodes= — like AllowedMemoryNodes= but only
    /// during startup. See systemd.resource-control(5).
    pub startup_allowed_memory_nodes: Option<String>,

    /// DefaultMemoryMin= — default memory.min for children of this unit's
    /// cgroup. See systemd.resource-control(5).
    pub default_memory_min: Option<MemoryLimit>,

    /// DefaultMemoryLow= — default memory.low for children of this unit's
    /// cgroup. See systemd.resource-control(5).
    pub default_memory_low: Option<MemoryLimit>,

    /// MemoryZSwapMax= — hard zswap limit for the unit's cgroup. Accepts
    /// a byte value with optional K/M/G/T/P/E suffix, a percentage, or
    /// "infinity". Applied to cgroup memory.zswap.max at runtime.
    /// See systemd.resource-control(5).
    pub memory_zswap_max: Option<MemoryLimit>,

    /// IODeviceLatencyTargetSec= — per-device I/O latency target (format:
    /// "/dev/path TIMESPAN"). Applied to cgroup io.latency at runtime.
    /// See systemd.resource-control(5).
    pub io_device_latency_target_sec: Vec<String>,

    /// DisableControllers= — a space-separated list of cgroup controllers
    /// to disable for the unit. See systemd.resource-control(5).
    pub disable_controllers: Vec<String>,

    /// MemoryPressureThresholdSec= — memory pressure threshold duration.
    /// Used with MemoryPressureWatch=. See systemd.resource-control(5).
    pub memory_pressure_threshold_sec: Option<Timeout>,

    /// IPIngressFilterPath= — absolute path to a pinned BPF program for
    /// ingress IP packet filtering. Multiple directives accumulate; an
    /// empty assignment resets. See systemd.resource-control(5).
    pub ip_ingress_filter_path: Vec<String>,

    /// IPEgressFilterPath= — absolute path to a pinned BPF program for
    /// egress IP packet filtering. Multiple directives accumulate; an
    /// empty assignment resets. See systemd.resource-control(5).
    pub ip_egress_filter_path: Vec<String>,

    /// BPFProgram= — attach a BPF program to the unit's cgroup. Format:
    /// "type:path". Multiple directives accumulate; an empty assignment
    /// resets. See systemd.resource-control(5).
    pub bpf_program: Vec<String>,

    /// SocketBindAllow= — allow binding to specific socket address families,
    /// protocols, and ports. Format: "bind-rule". Multiple directives
    /// accumulate; an empty assignment resets. See systemd.resource-control(5).
    pub socket_bind_allow: Vec<String>,

    /// SocketBindDeny= — deny binding to specific socket address families,
    /// protocols, and ports. Format: "bind-rule". Multiple directives
    /// accumulate; an empty assignment resets. See systemd.resource-control(5).
    pub socket_bind_deny: Vec<String>,

    /// RestrictNetworkInterfaces= — restrict network interfaces accessible
    /// to the unit. Accepts a space-separated list of interface names
    /// (prefixed with "~" for deny). Multiple directives accumulate; an
    /// empty assignment resets. See systemd.resource-control(5).
    pub restrict_network_interfaces: Vec<String>,

    /// NFTSet= — attach the unit's cgroup to an nftables set. Format:
    /// "family:table:set". Multiple directives accumulate; an empty
    /// assignment resets. See systemd.resource-control(5).
    pub nft_set: Vec<String>,

    pub exec_section: ParsedExecSection,
}

#[derive(Default)]
pub struct ParsedInstallSection {
    pub wanted_by: Vec<String>,
    pub required_by: Vec<String>,
    pub also: Vec<String>,
    pub alias: Vec<String>,

    /// Default instance name for template units (e.g. `foo@.service`).
    /// When a template is enabled without an explicit instance, this value is used.
    /// Matches systemd's `DefaultInstance=` setting in the `[Install]` section.
    pub default_instance: Option<String>,
}
pub struct ParsedExecSection {
    pub user: Option<String>,
    pub group: Option<String>,
    pub stdin_option: StandardInput,
    pub stdout_path: Option<StdIoOption>,
    pub stderr_path: Option<StdIoOption>,
    pub supplementary_groups: Vec<String>,
    pub environment: Option<EnvVars>,
    /// Paths from EnvironmentFile= directives. A leading '-' means the file
    /// is optional (no error if it doesn't exist).
    pub environment_files: Vec<(PathBuf, bool)>,
    pub working_directory: Option<PathBuf>,
    pub state_directory: Vec<String>,
    /// LogsDirectory= — directories to create under /var/log/ before the
    /// service starts. Ownership is set to the service user/group and the
    /// LOGS_DIRECTORY environment variable is set to a colon-separated
    /// list of the absolute paths. Matches systemd.exec(5).
    pub logs_directory: Vec<String>,
    /// LogsDirectoryMode= — the file system access mode to use when creating
    /// the logs directories specified with LogsDirectory=. Takes an access
    /// mode in octal notation (e.g. 0755). Defaults to 0755.
    /// Parsed and stored; runtime enforcement applies the mode at directory
    /// creation time. See systemd.exec(5).
    pub logs_directory_mode: Option<u32>,
    /// RuntimeDirectory= — directories to create under /run/ before the
    /// service starts. Ownership is set to the service user/group and the
    /// RUNTIME_DIRECTORY environment variable is set to a colon-separated
    /// list of the absolute paths. Matches systemd.exec(5).
    pub runtime_directory: Vec<String>,
    /// RuntimeDirectoryPreserve= — controls whether runtime directories
    /// (created by `RuntimeDirectory=`) are removed when the service stops.
    /// Can be `no` (default, always remove), `yes` (always preserve), or
    /// `restart` (preserve across restarts, remove on full stop). Parsed and
    /// stored; no runtime enforcement yet. See systemd.exec(5).
    pub runtime_directory_preserve: RuntimeDirectoryPreserve,
    pub tty_path: Option<PathBuf>,
    /// TTYReset= — reset the TTY to sane defaults before use (default: false).
    /// Matches systemd behavior: resets termios, keyboard mode, switches to text mode.
    pub tty_reset: bool,
    /// TTYVHangup= — send TIOCVHANGUP to the TTY before use (default: false).
    /// This disconnects any prior sessions from the TTY so the new service gets
    /// a clean controlling terminal.
    pub tty_vhangup: bool,
    /// TTYVTDisallocate= — deallocate or clear the VT before use (default: false).
    pub tty_vt_disallocate: bool,
    /// IgnoreSIGPIPE= — if true (the default), SIGPIPE is set to SIG_IGN before
    /// exec'ing the service binary. When false, the default SIGPIPE disposition
    /// (terminate) is left in place. Matches systemd.exec(5).
    pub ignore_sigpipe: bool,
    /// UtmpIdentifier= — the 4-character identifier string to write to the utmp
    /// and wtmp entries when the service runs on a TTY. Defaults to the TTY
    /// basename when unset. See systemd.exec(5).
    pub utmp_identifier: Option<String>,
    /// UtmpMode= — the type of utmp/wtmp record to write. Defaults to `Init`.
    /// See systemd.exec(5).
    pub utmp_mode: UtmpMode,
    /// ImportCredential= — glob patterns for credentials to import from the
    /// system credential store into the service's credential directory.
    /// Multiple patterns may be specified (the setting accumulates).
    /// See systemd.exec(5).
    pub import_credentials: Vec<String>,
    /// LoadCredential=ID:PATH — load a credential from a file. The credential
    /// is made available in the service's credential directory under the given
    /// ID name. Multiple directives accumulate; an empty assignment resets.
    /// See systemd.exec(5).
    pub load_credentials: Vec<(String, String)>,
    /// LoadCredentialEncrypted=ID:PATH — like LoadCredential= but the file
    /// content is expected to be encrypted with a system credential key.
    /// Parsed but decryption is not yet implemented (credential is loaded
    /// as-is). See systemd.exec(5).
    pub load_credentials_encrypted: Vec<(String, String)>,
    /// SetCredential=ID:DATA — set a credential from inline data. The data
    /// is written verbatim to the credential directory under the given ID.
    /// The DATA part may contain colons. Multiple directives accumulate;
    /// an empty assignment resets. See systemd.exec(5).
    pub set_credentials: Vec<(String, String)>,
    /// SetCredentialEncrypted=ID:DATA — like SetCredential= but the inline
    /// data is expected to be encrypted (base64-encoded). Parsed but
    /// decryption is not yet implemented. See systemd.exec(5).
    pub set_credentials_encrypted: Vec<(String, String)>,
    /// PassEnvironment= — a list of environment variable names to import from
    /// the system manager's (PID 1) environment into the service's execution
    /// environment. Only variables that are actually set in the manager's
    /// environment are passed; unset variables are silently ignored. Multiple
    /// directives accumulate; an empty assignment resets the list. Applied
    /// after Environment= but before internal variables and UnsetEnvironment=.
    /// See systemd.exec(5).
    pub pass_environment: Vec<String>,
    /// UnsetEnvironment= — a list of environment variable names or variable
    /// assignments (VAR=VALUE) to remove from the final environment passed to
    /// executed processes. If a plain name is given, any assignment with that
    /// name is removed regardless of value. If a VAR=VALUE assignment is given,
    /// only an exact match is removed. Applied as the final step when
    /// compiling the environment block. See systemd.exec(5).
    pub unset_environment: Vec<String>,
    /// OOMScoreAdjust= — sets the OOM score adjustment for executed processes.
    /// Takes an integer between -1000 (least likely to be killed) and 1000
    /// (most likely to be killed). Written to /proc/self/oom_score_adj before
    /// exec. See systemd.exec(5).
    pub oom_score_adjust: Option<i32>,
    /// LogExtraFields= — additional journal fields to include in log entries
    /// for this unit. Each entry is a KEY=VALUE string. Multiple directives
    /// accumulate. Parsed and stored; not yet used at runtime. See systemd.exec(5).
    pub log_extra_fields: Vec<String>,
    /// DynamicUser= — if true, a UNIX user and group pair is dynamically
    /// allocated for this unit at runtime and released when the unit is stopped.
    /// Defaults to false. See systemd.exec(5).
    pub dynamic_user: bool,
    /// SystemCallFilter= — a list of syscall names or `@group` names for
    /// seccomp-based system-call filtering. Entries prefixed with `~` form a
    /// deny-list; without the prefix they form an allow-list. Multiple
    /// directives accumulate; an empty assignment resets the list. Parsed and
    /// stored; no runtime enforcement yet. See systemd.exec(5).
    pub system_call_filter: Vec<String>,
    /// SystemCallLog= — a list of syscall names or `@group` names for
    /// seccomp-based system-call logging. Matching system calls are logged
    /// to the audit log (if available) without blocking them. Same syntax
    /// as `SystemCallFilter=`: entries prefixed with `~` exclude from
    /// logging. Multiple directives accumulate; an empty assignment resets
    /// the list. Parsed and stored; no runtime seccomp audit enforcement
    /// yet. See systemd.exec(5).
    pub system_call_log: Vec<String>,
    /// RestrictFileSystems= — a list of Linux file system type names
    /// (e.g. `ext4`, `tmpfs`, `proc`, `btrfs`) that processes of this
    /// unit may access. If prefixed with `~`, the listed file systems
    /// are denied instead. Requires BPF LSM support in the kernel.
    /// Multiple directives accumulate; an empty assignment resets the
    /// list. Parsed and stored; no runtime BPF LSM enforcement yet.
    /// See systemd.exec(5).
    pub restrict_file_systems: Vec<String>,
    /// ProtectSystem= — controls whether the service has read-only access to
    /// the OS file system hierarchy. Parsed and stored; no runtime enforcement
    /// yet (requires mount namespace support). See systemd.exec(5).
    pub protect_system: ProtectSystem,
    /// RestrictNamespaces= — restricts access to Linux namespace types for the
    /// service. Can be a boolean (`yes` restricts all, `no` allows all) or a
    /// space-separated list of namespace type identifiers (cgroup, ipc, net,
    /// mnt, pid, user, uts). A `~` prefix inverts the list. Parsed and stored;
    /// no runtime seccomp enforcement yet. See systemd.exec(5).
    pub restrict_namespaces: RestrictNamespaces,
    /// RestrictRealtime= — if true, any attempts to enable realtime scheduling
    /// in a process of the unit are refused via seccomp. Defaults to false.
    /// Parsed and stored; no runtime seccomp enforcement yet. See systemd.exec(5).
    pub restrict_realtime: bool,
    /// RestrictAddressFamilies= — a list of address family names (e.g.
    /// AF_UNIX, AF_INET, AF_INET6) for seccomp-based socket address family
    /// filtering. Entries prefixed with `~` form a deny-list; without the
    /// prefix they form an allow-list. Multiple directives accumulate; an
    /// empty assignment resets the list. Parsed and stored; no runtime seccomp
    /// enforcement yet. See systemd.exec(5).
    pub restrict_address_families: Vec<String>,
    /// SystemCallErrorNumber= — the errno to return when a system call is
    /// blocked by `SystemCallFilter=`. Takes an errno name such as `EPERM`
    /// or `EACCES`. When not set, blocked calls result in SIGSYS (process
    /// kill). Parsed and stored; no runtime seccomp enforcement yet.
    /// See systemd.exec(5).
    pub system_call_error_number: Option<String>,
    /// NoNewPrivileges= — if true, ensures that the service process and all
    /// its children can never gain new privileges through execve() (e.g.
    /// via setuid/setgid bits or file capabilities). Defaults to false.
    /// Parsed and stored; no runtime enforcement yet. See systemd.exec(5).
    pub no_new_privileges: bool,
    /// ProtectControlGroups= — if true, the Linux Control Groups (cgroups)
    /// hierarchies accessible through /sys/fs/cgroup/ will be made read-only
    /// to all processes of the unit. Defaults to false. Parsed and stored;
    /// no runtime enforcement yet (requires mount namespace support).
    /// See systemd.exec(5).
    pub protect_control_groups: bool,
    /// ProtectKernelModules= — if true, explicit kernel module loading and
    /// unloading is denied. This also makes /usr/lib/modules/ inaccessible.
    /// Defaults to false. Parsed and stored; no runtime enforcement yet
    /// (requires mount namespace and seccomp support). See systemd.exec(5).
    pub protect_kernel_modules: bool,
    /// RestrictSUIDSGID= — if true, any attempts to set the set-user-ID
    /// (SUID) or set-group-ID (SGID) bits on files or directories will be
    /// denied. Defaults to false. Parsed and stored; no runtime enforcement
    /// yet (requires seccomp support). See systemd.exec(5).
    pub restrict_suid_sgid: bool,
    /// ProtectKernelLogs= — if true, access to the kernel log ring buffer
    /// (/dev/kmsg, /proc/kmsg, dmesg) is denied. Defaults to false. Parsed
    /// and stored; no runtime enforcement yet (requires mount namespace and
    /// seccomp support). See systemd.exec(5).
    pub protect_kernel_logs: bool,
    /// ProtectKernelTunables= — if true, kernel variables accessible through
    /// /proc/sys/, /sys/, /proc/sysrq-trigger, /proc/latency_stats,
    /// /proc/acpi, /proc/timer_stats, /proc/fs, and /proc/irq will be made
    /// read-only to all processes of the unit. Defaults to false. Parsed and
    /// stored; no runtime enforcement yet (requires mount namespace support).
    /// See systemd.exec(5).
    pub protect_kernel_tunables: bool,
    /// ProtectClock= — if true, writes to the system and hardware clock are
    /// denied. Defaults to false. Parsed and stored; no runtime enforcement
    /// yet (requires seccomp and device access restrictions).
    /// See systemd.exec(5).
    pub protect_clock: bool,
    /// CapabilityBoundingSet= — a list of Linux capability names (e.g.
    /// CAP_NET_ADMIN, CAP_SYS_PTRACE) controlling the capability bounding
    /// set for executed processes. Entries prefixed with `~` form a deny-list;
    /// without the prefix they form an allow-list. Multiple directives
    /// accumulate; an empty assignment resets the list. Parsed and stored;
    /// no runtime enforcement yet. See systemd.exec(5).
    pub capability_bounding_set: Vec<String>,
    /// AmbientCapabilities= — a list of Linux capability names (e.g.
    /// CAP_NET_BIND_SERVICE, CAP_SYS_NICE) controlling the ambient
    /// capability set for executed processes. Ambient capabilities are
    /// inherited across execve() even without file capabilities. Entries
    /// prefixed with `~` form a deny-list; without the prefix they form an
    /// allow-list. Multiple directives accumulate; an empty assignment resets
    /// the list. Parsed and stored; no runtime enforcement yet.
    /// See systemd.exec(5).
    pub ambient_capabilities: Vec<String>,
    /// ProtectHome= — controls whether /home, /root, and /run/user are
    /// accessible to the service. Can be `no` (default), `yes` (inaccessible),
    /// `read-only`, or `tmpfs`. Parsed and stored; no runtime enforcement yet
    /// (requires mount namespace support). See systemd.exec(5).
    pub protect_home: ProtectHome,
    /// ProtectHostname= — if true, sets up a new UTS namespace for the
    /// service and prevents it from changing the hostname or domainname.
    /// Defaults to false. Parsed and stored; no runtime enforcement yet
    /// (requires UTS namespace and seccomp support). See systemd.exec(5).
    pub protect_hostname: bool,
    /// SystemCallArchitectures= — a space-separated list of architecture
    /// identifiers (e.g. `native`, `x86`, `x86-64`) restricting which
    /// architectures system calls may be invoked from. Multiple directives
    /// accumulate; an empty assignment resets the list. Parsed and stored;
    /// no runtime seccomp enforcement yet. See systemd.exec(5).
    pub system_call_architectures: Vec<String>,
    /// ReadWritePaths= — a space-separated list of file system paths that
    /// should be made accessible read-write for the service, even if other
    /// settings (like `ProtectSystem=strict`) would otherwise make them
    /// read-only. Multiple directives accumulate; an empty assignment resets
    /// the list. Parsed and stored; no runtime mount-namespace enforcement
    /// yet. See systemd.exec(5).
    pub read_write_paths: Vec<String>,
    /// MemoryDenyWriteExecute= — if true, attempts to create memory mappings
    /// that are both writable and executable, or to change existing writable
    /// mappings to executable, are prohibited. Defaults to false. Parsed and
    /// stored; no runtime seccomp enforcement yet. See systemd.exec(5).
    pub memory_deny_write_execute: bool,
    /// LockPersonality= — if true, locks down the personality(2) system call
    /// so that the kernel execution domain may not be changed from the default
    /// or the personality selected with Personality=. Defaults to false.
    /// Parsed and stored; no runtime seccomp enforcement yet. See systemd.exec(5).
    pub lock_personality: bool,
    /// ProtectProc= — controls the `hidepid=` mount option of the procfs
    /// instance for the unit. Takes one of "default", "noaccess", "invisible",
    /// or "ptraceable". Defaults to "default". Parsed and stored; no runtime
    /// mount-namespace enforcement yet. See systemd.exec(5).
    pub protect_proc: ProtectProc,
    /// PrivateTmp= — if true, sets up a private /tmp and /var/tmp namespace
    /// for the executed processes. Files in these directories are not visible
    /// to other processes and vice versa. Defaults to false. Parsed and
    /// stored; no runtime mount-namespace enforcement yet. See systemd.exec(5).
    pub private_tmp: bool,
    /// PrivateDevices= — if true, sets up a private /dev/ namespace for the
    /// executed processes with only pseudo-devices such as /dev/null,
    /// /dev/zero, /dev/random available. Physical devices are not accessible.
    /// Defaults to false. Parsed and stored; no runtime mount-namespace
    /// enforcement yet. See systemd.exec(5).
    pub private_devices: bool,
    /// PrivateNetwork= — if true, sets up a new network namespace for the
    /// executed processes with only the loopback device "lo" (and no
    /// external network connectivity). Defaults to false. Parsed and stored;
    /// no runtime namespace enforcement yet. See systemd.exec(5).
    pub private_network: bool,
    /// PrivateUsers= — if true, sets up a new user namespace for the
    /// executed processes and configures a minimal user/group mapping:
    /// the root user/group as well as the unit's own user/group are
    /// mapped to themselves, everything else is mapped to the "nobody"
    /// user/group. Defaults to false. Parsed and stored; no runtime
    /// user-namespace enforcement yet. See systemd.exec(5).
    pub private_users: bool,
    /// PrivateMounts= — if true, the processes of this unit will be run in
    /// their own private file system (mount) namespace with all mount
    /// propagation from the processes towards the host's main file system
    /// namespace turned off. File system mount points established or removed
    /// on the host will still be propagated to the unit's processes.
    /// Defaults to false. Parsed and stored; no runtime mount-namespace
    /// enforcement yet. See systemd.exec(5).
    pub private_mounts: bool,
    /// IOSchedulingClass= — sets the I/O scheduling class for executed
    /// processes. Takes one of "none" (or "0"), "realtime" (or "1"),
    /// "best-effort" (or "2"), or "idle" (or "3"). Defaults to None
    /// (kernel default, which is best-effort). Parsed and stored; no
    /// runtime ioprio_set() enforcement yet. See systemd.exec(5).
    pub io_scheduling_class: IOSchedulingClass,
    /// IOSchedulingPriority= — sets the I/O scheduling priority for executed
    /// processes. Takes an integer between 0 (highest priority) and 7
    /// (lowest priority). The default priority for the best-effort scheduling
    /// class is 4. Parsed and stored; no runtime enforcement yet.
    /// See systemd.exec(5).
    pub io_scheduling_priority: Option<u8>,
    /// UMask= — sets the file mode creation mask (umask) for executed
    /// processes. Takes an octal value (e.g. 0022, 0077). Defaults to 0022.
    /// Parsed and stored; no runtime enforcement yet. See systemd.exec(5).
    pub umask: Option<u32>,
    /// ProcSubset= — controls which subset of /proc/ is mounted for the
    /// unit. Takes one of "all" (full /proc, default) or "pid" (only
    /// process-specific subdirectories). Parsed and stored; no runtime
    /// mount-namespace enforcement yet. See systemd.exec(5).
    pub proc_subset: ProcSubset,
    /// Nice= — sets the default nice level (scheduling priority) for
    /// executed processes. Takes an integer between -20 (highest priority)
    /// and 19 (lowest priority). Parsed and stored; no runtime enforcement
    /// yet. See systemd.exec(5).
    pub nice: Option<i32>,
    /// RemoveIPC= — if true, all System V and POSIX IPC objects owned by
    /// the user and group of the executed processes are removed when the
    /// unit is stopped. Defaults to false. Parsed and stored; no runtime
    /// enforcement yet. See systemd.exec(5).
    pub remove_ipc: bool,
    /// PAMName= — sets the PAM service name to set up a session as.
    /// If set, PAM authentication, account management, and session open/close
    /// calls are performed for the executed processes. Takes a string such as
    /// "login", "systemd-user", or "sshd". Defaults to None (no PAM session).
    /// Parsed and stored; no runtime PAM enforcement yet. See systemd.exec(5).
    pub pam_name: Option<String>,

    // ── Resource limits (LimitXXX=) ──────────────────────────────────
    // All Limit* directives follow the same format: a numeric value, a
    // soft:hard pair, or "infinity". Applied via setrlimit() before exec.
    // See systemd.exec(5).
    /// LimitCPU= — maximum CPU time in seconds. Applied via
    /// `setrlimit(RLIMIT_CPU)`. See systemd.exec(5).
    pub limit_cpu: Option<ResourceLimit>,
    /// LimitCORE= — maximum size of core dump files. Applied via
    /// `setrlimit(RLIMIT_CORE)`. See systemd.exec(5).
    pub limit_core: Option<ResourceLimit>,
    /// LimitFSIZE= — maximum size of files the process may create. Applied
    /// via `setrlimit(RLIMIT_FSIZE)`. See systemd.exec(5).
    pub limit_fsize: Option<ResourceLimit>,
    /// LimitDATA= — maximum size of the process's data segment. Applied via
    /// `setrlimit(RLIMIT_DATA)`. See systemd.exec(5).
    pub limit_data: Option<ResourceLimit>,
    /// LimitSTACK= — maximum size of the process stack. Applied via
    /// `setrlimit(RLIMIT_STACK)`. See systemd.exec(5).
    pub limit_stack: Option<ResourceLimit>,
    /// LimitRSS= — maximum resident set size. Applied via
    /// `setrlimit(RLIMIT_RSS)`. See systemd.exec(5).
    pub limit_rss: Option<ResourceLimit>,
    /// LimitNPROC= — maximum number of processes for the real user ID.
    /// Applied via `setrlimit(RLIMIT_NPROC)`. See systemd.exec(5).
    pub limit_nproc: Option<ResourceLimit>,
    /// LimitMEMLOCK= — maximum number of bytes of memory that may be locked.
    /// Applied via `setrlimit(RLIMIT_MEMLOCK)`. See systemd.exec(5).
    pub limit_memlock: Option<ResourceLimit>,
    /// LimitAS= — maximum size of the process's virtual memory (address
    /// space). Applied via `setrlimit(RLIMIT_AS)`. See systemd.exec(5).
    pub limit_as: Option<ResourceLimit>,
    /// LimitLOCKS= — maximum number of flock/fcntl locks. Applied via
    /// `setrlimit(RLIMIT_LOCKS)`. See systemd.exec(5).
    pub limit_locks: Option<ResourceLimit>,
    /// LimitSIGPENDING= — maximum number of signals that may be queued.
    /// Applied via `setrlimit(RLIMIT_SIGPENDING)`. See systemd.exec(5).
    pub limit_sigpending: Option<ResourceLimit>,
    /// LimitMSGQUEUE= — maximum number of bytes in POSIX message queues.
    /// Applied via `setrlimit(RLIMIT_MSGQUEUE)`. See systemd.exec(5).
    pub limit_msgqueue: Option<ResourceLimit>,
    /// LimitNICE= — maximum nice priority allowed to raise to (range 0–40,
    /// mapped to nice values 20..-20). Applied via `setrlimit(RLIMIT_NICE)`.
    /// See systemd.exec(5).
    pub limit_nice: Option<ResourceLimit>,
    /// LimitRTPRIO= — maximum realtime scheduling priority. Applied via
    /// `setrlimit(RLIMIT_RTPRIO)`. See systemd.exec(5).
    pub limit_rtprio: Option<ResourceLimit>,
    /// LimitRTTIME= — maximum number of microseconds a realtime-scheduled
    /// process may consume without blocking. Applied via
    /// `setrlimit(RLIMIT_RTTIME)`. See systemd.exec(5).
    pub limit_rttime: Option<ResourceLimit>,

    // ── Directory management ─────────────────────────────────────────
    /// CacheDirectory= — directories to create under /var/cache/ before the
    /// service starts. Ownership is set to the service user/group and the
    /// CACHE_DIRECTORY environment variable is set to a colon-separated
    /// list of the absolute paths. Matches systemd.exec(5).
    pub cache_directory: Vec<String>,
    /// CacheDirectoryMode= — the file system access mode to use when creating
    /// the cache directories specified with CacheDirectory=. Takes an access
    /// mode in octal notation (e.g. 0755). Defaults to 0755.
    /// See systemd.exec(5).
    pub cache_directory_mode: Option<u32>,
    /// ConfigurationDirectory= — directories to create under /etc/ before the
    /// service starts. Ownership is set to the service user/group and the
    /// CONFIGURATION_DIRECTORY environment variable is set to a
    /// colon-separated list of the absolute paths. Matches systemd.exec(5).
    pub configuration_directory: Vec<String>,
    /// ConfigurationDirectoryMode= — the file system access mode to use when
    /// creating the configuration directories specified with
    /// ConfigurationDirectory=. Takes an access mode in octal notation
    /// (e.g. 0755). Defaults to 0755. See systemd.exec(5).
    pub configuration_directory_mode: Option<u32>,
    /// StateDirectoryMode= — the file system access mode to use when creating
    /// the state directories specified with StateDirectory=. Takes an access
    /// mode in octal notation (e.g. 0755). Defaults to 0755.
    /// See systemd.exec(5).
    pub state_directory_mode: Option<u32>,
    /// RuntimeDirectoryMode= — the file system access mode to use when
    /// creating the runtime directories specified with RuntimeDirectory=.
    /// Takes an access mode in octal notation (e.g. 0755). Defaults to 0755.
    /// See systemd.exec(5).
    pub runtime_directory_mode: Option<u32>,

    // ── Path-based mount namespace directives ────────────────────────
    /// ReadOnlyPaths= — a space-separated list of file system paths that
    /// should be made read-only for the service within its mount namespace.
    /// Multiple directives accumulate; an empty assignment resets the list.
    /// See systemd.exec(5).
    pub read_only_paths: Vec<String>,
    /// InaccessiblePaths= — a space-separated list of file system paths that
    /// should be made completely inaccessible (hidden) for the service within
    /// its mount namespace. Multiple directives accumulate; an empty
    /// assignment resets the list. See systemd.exec(5).
    pub inaccessible_paths: Vec<String>,
    /// BindPaths= — bind-mount host paths into the service's mount namespace
    /// (read-write). Each entry is "SOURCE[:DEST[:OPTIONS]]". Multiple
    /// directives accumulate; an empty assignment resets the list.
    /// See systemd.exec(5).
    pub bind_paths: Vec<String>,
    /// BindReadOnlyPaths= — bind-mount host paths into the service's mount
    /// namespace (read-only). Same format as BindPaths=. Multiple directives
    /// accumulate; an empty assignment resets the list. See systemd.exec(5).
    pub bind_read_only_paths: Vec<String>,
    /// TemporaryFileSystem= — mount tmpfs file systems on the specified paths
    /// within the service's mount namespace. Each entry is "PATH[:OPTIONS]".
    /// Multiple directives accumulate; an empty assignment resets the list.
    /// See systemd.exec(5).
    pub temporary_file_system: Vec<String>,

    // ── Logging directives ───────────────────────────────────────────
    /// SyslogIdentifier= — sets the process name ("tag") to prefix log
    /// messages with. Defaults to the name of the executed process.
    /// See systemd.exec(5).
    pub syslog_identifier: Option<String>,

    // ── Additional logging directives ────────────────────────────────
    /// SyslogFacility= — sets the syslog facility identifier to use when
    /// logging to syslog. Takes one of `kern`, `user`, `mail`, `daemon`,
    /// `auth`, `syslog`, `lpr`, `news`, `uucp`, `cron`, `authpriv`, `ftp`,
    /// `local0`–`local7`. Defaults to `daemon`. See systemd.exec(5).
    pub syslog_facility: Option<String>,
    /// SyslogLevel= — sets the default syslog level to use when logging to
    /// syslog. Takes one of `emerg`, `alert`, `crit`, `err`, `warning`,
    /// `notice`, `info`, `debug`. Defaults to `info`. See systemd.exec(5).
    pub syslog_level: Option<String>,
    /// SyslogLevelPrefix= — if true (default), syslog-style log level prefix
    /// strings (`<N>`) at the beginning of log lines are interpreted and
    /// stripped from the log message. See systemd.exec(5).
    pub syslog_level_prefix: Option<bool>,
    /// LogLevelMax= — filters log messages by level: messages with a log
    /// level higher (i.e. less important) than the specified level are
    /// dropped. Takes the same values as `SyslogLevel=`. See systemd.exec(5).
    pub log_level_max: Option<String>,
    /// LogRateLimitIntervalSec= — configures the rate limiting interval for
    /// messages logged by this unit. If more than `LogRateLimitBurst=`
    /// messages are logged within this interval, further messages are dropped
    /// until the interval is over. Takes a time span value. Defaults to
    /// `30s`. See systemd.exec(5).
    pub log_rate_limit_interval_sec: Option<String>,
    /// LogRateLimitBurst= — configures the burst limit for rate limiting of
    /// log messages. If more than this number of messages are logged within
    /// the interval configured with `LogRateLimitIntervalSec=`, further
    /// messages are dropped. Defaults to `10000`. See systemd.exec(5).
    pub log_rate_limit_burst: Option<u32>,
    /// LogFilterPatterns= — defines a list of extended regular expression
    /// patterns for filtering log messages. Messages matching `~PATTERN`
    /// entries are discarded; messages matching plain `PATTERN` entries are
    /// kept. Multiple directives accumulate; an empty assignment resets.
    /// See systemd.exec(5).
    pub log_filter_patterns: Vec<String>,
    /// LogNamespace= — run the service in the specified journal namespace.
    /// The journal namespace name is used to build a unique journal socket
    /// path, allowing log isolation. See systemd.exec(5).
    pub log_namespace: Option<String>,

    // ── CPU scheduling directives ────────────────────────────────────
    /// CPUSchedulingPolicy= — sets the CPU scheduling policy for executed
    /// processes. Takes one of `other`, `batch`, `idle`, `fifo`, or `rr`.
    /// See systemd.exec(5).
    pub cpu_scheduling_policy: Option<String>,
    /// CPUSchedulingPriority= — sets the CPU scheduling priority for
    /// executed processes. The available priority range depends on the
    /// selected CPU scheduling policy. For `fifo` and `rr`, the range is
    /// 1 (lowest) to 99 (highest). See systemd.exec(5).
    pub cpu_scheduling_priority: Option<u32>,
    /// CPUSchedulingResetOnFork= — if true, elevated CPU scheduling
    /// priorities and policies are reset when the executed processes fork.
    /// Defaults to false. See systemd.exec(5).
    pub cpu_scheduling_reset_on_fork: Option<bool>,
    /// CPUAffinity= — sets the CPU affinity for executed processes. Takes a
    /// list of CPU indices or ranges separated by spaces or commas (e.g.
    /// `0 1 3` or `0-3 8-11`). Special value `numa` uses NUMA node CPUs.
    /// Multiple directives accumulate; empty resets. See systemd.exec(5).
    pub cpu_affinity: Vec<String>,
    /// NUMAPolicy= — sets the NUMA memory allocation policy for the
    /// executed processes. Takes one of `default`, `preferred`, `bind`,
    /// `interleave`, or `local`. See systemd.exec(5).
    pub numa_policy: Option<String>,
    /// NUMAMask= — sets the NUMA node mask for the policy configured with
    /// `NUMAPolicy=`. Takes a list of NUMA node indices or ranges.
    /// See systemd.exec(5).
    pub numa_mask: Option<String>,

    // ── Root filesystem / image directives ───────────────────────────
    /// RootDirectory= — sets the root directory for executed processes,
    /// using `pivot_root(2)` or `chroot(2)`. See systemd.exec(5).
    pub root_directory: Option<String>,
    /// RootImage= — sets the root file system image to mount as the root
    /// directory for executed processes. Takes a path to a disk image file
    /// or block device. See systemd.exec(5).
    pub root_image: Option<String>,
    /// RootImageOptions= — mount options for partitions within the root
    /// image. Each entry is `PARTITION_NAME:OPTIONS`. Multiple directives
    /// accumulate; an empty assignment resets. See systemd.exec(5).
    pub root_image_options: Vec<String>,
    /// RootHash= — takes a dm-verity data root hash for the root image
    /// specified with `RootImage=`. See systemd.exec(5).
    pub root_hash: Option<String>,
    /// RootHashSignature= — takes a dm-verity root hash signature for the
    /// root image, as either a path to a DER-encoded PKCS#7 signature file
    /// or as a `base64:` prefixed inline signature. See systemd.exec(5).
    pub root_hash_signature: Option<String>,
    /// RootVerity= — takes a path to a dm-verity data file for the root
    /// image specified with `RootImage=`. See systemd.exec(5).
    pub root_verity: Option<String>,
    /// RootEphemeral= — if true, an ephemeral snapshot of the root image
    /// is created and used. Changes are lost on service stop.
    /// See systemd.exec(5).
    pub root_ephemeral: Option<bool>,
    /// MountAPIVFS= — if true, /proc/, /sys/, /dev/ and /tmp/ are mounted
    /// inside the root directory or image before the service starts. Defaults
    /// to off unless `RootDirectory=` or `RootImage=` is used.
    /// See systemd.exec(5).
    pub mount_api_vfs: Option<bool>,
    /// ExtensionDirectories= — a list of directories containing system
    /// extension hierarchy trees to overlay on top of the root filesystem.
    /// Multiple directives accumulate; empty resets. See systemd.exec(5).
    pub extension_directories: Vec<String>,
    /// ExtensionImages= — a list of disk image paths containing system
    /// extension hierarchies to overlay on top of the root filesystem.
    /// Multiple directives accumulate; empty resets. See systemd.exec(5).
    pub extension_images: Vec<String>,
    /// MountImages= — a list of disk image paths to mount at specified
    /// mount points within the service's file system namespace. Each entry
    /// is `SOURCE:DEST[:OPTIONS]`. Multiple directives accumulate; empty
    /// resets. See systemd.exec(5).
    pub mount_images: Vec<String>,
    /// BindLogSockets= — if true, the journal logging sockets are bound
    /// into the service's mount namespace even if it uses `RootImage=` or
    /// `RootDirectory=`. See systemd.exec(5).
    pub bind_log_sockets: Option<bool>,

    // ── Additional namespace directives ──────────────────────────────
    /// PrivateIPC= — if true, sets up a new IPC namespace for the executed
    /// processes. Defaults to false. See systemd.exec(5).
    pub private_ipc: Option<bool>,
    /// PrivatePIDs= — if true, sets up a new PID namespace for the executed
    /// processes. Defaults to false. Added in systemd 257.
    /// See systemd.exec(5).
    pub private_pids: Option<bool>,
    /// IPCNamespacePath= — run the service in the specified existing IPC
    /// namespace. Takes a file path to an IPC namespace file (usually
    /// under /proc/PID/ns/ipc). See systemd.exec(5).
    pub ipc_namespace_path: Option<String>,
    /// NetworkNamespacePath= — run the service in the specified existing
    /// network namespace. Takes a file path to a network namespace file
    /// (usually under /proc/PID/ns/net or /run/netns/NAME).
    /// See systemd.exec(5).
    pub network_namespace_path: Option<String>,

    // ── Security directives ──────────────────────────────────────────
    /// SecureBits= — controls the secure-bits flags of the executed
    /// process. Takes a space-separated combination of `keep-caps`,
    /// `keep-caps-locked`, `no-setuid-fixup`, `no-setuid-fixup-locked`,
    /// `noroot`, `noroot-locked`. See systemd.exec(5).
    pub secure_bits: Vec<String>,
    /// Personality= — controls the process personality / execution domain.
    /// Takes one of `x86`, `x86-64`, `ppc`, `ppc-le`, `ppc64`, `ppc64-le`,
    /// `s390`, `s390x`, etc. On x86-64 hosts, setting `x86` enables 32-bit
    /// compatibility mode. See systemd.exec(5).
    pub personality: Option<String>,
    /// SELinuxContext= — sets the SELinux security context for executed
    /// processes. A `-` prefix makes the setting optional (no error if
    /// SELinux is not available). See systemd.exec(5).
    pub selinux_context: Option<String>,
    /// AppArmorProfile= — sets the AppArmor profile for executed processes.
    /// A `-` prefix makes the setting optional (no error if AppArmor is not
    /// available). See systemd.exec(5).
    pub apparmor_profile: Option<String>,
    /// SmackProcessLabel= — sets the SMACK security label for executed
    /// processes. A `-` prefix makes the setting optional. See systemd.exec(5).
    pub smack_process_label: Option<String>,
    /// KeyringMode= — controls the kernel keyring set up for the service.
    /// Takes one of `inherit`, `private` (default for system services),
    /// or `shared`. See systemd.exec(5).
    pub keyring_mode: Option<String>,
    /// NoExecPaths= — a space-separated list of file system paths from
    /// which execution of programs is forbidden. Overrides ExecPaths=.
    /// Multiple directives accumulate; empty resets. See systemd.exec(5).
    pub no_exec_paths: Vec<String>,
    /// ExecPaths= — a space-separated list of file system paths from which
    /// execution of programs is permitted. Execution from all other paths
    /// is denied. Multiple directives accumulate; empty resets.
    /// See systemd.exec(5).
    pub exec_paths: Vec<String>,
    /// CoredumpFilter= — sets the coredump filter bitmask for the executed
    /// processes (written to /proc/PID/coredump_filter). Takes a
    /// space-separated combination of names (`private-anonymous`,
    /// `shared-anonymous`, `private-file-backed`, `shared-file-backed`,
    /// `elf-headers`, `private-huge`, `shared-huge`, `private-dax`,
    /// `shared-dax`) or a hexadecimal value. See systemd.exec(5).
    pub coredump_filter: Option<String>,

    // ── Misc directives ─────────────────────────────────────────────
    /// TimerSlackNSec= — sets the timer slack for executed processes,
    /// which controls the accuracy of wake-ups triggered by timers.
    /// Takes a time value in nanoseconds. See systemd.exec(5).
    pub timer_slack_nsec: Option<String>,
    /// StandardInputText= — configures arbitrary text data to pass via
    /// standard input to the executed process. Multiple directives
    /// accumulate (concatenated with newlines). C-style escapes are
    /// resolved. See systemd.exec(5).
    pub standard_input_text: Vec<String>,
    /// StandardInputData= — like StandardInputText=, but the data is
    /// specified in Base64 encoding. Useful for binary data.
    /// Multiple directives accumulate. See systemd.exec(5).
    pub standard_input_data: Vec<String>,
    /// SetLoginEnvironment= — if true, the `$XDG_SESSION_ID`,
    /// `$XDG_RUNTIME_DIR` and similar PAM login session environment
    /// variables are set. Defaults to unset (determined by `PAMName=`
    /// and service type). See systemd.exec(5).
    pub set_login_environment: Option<bool>,
}

/// The type of utmp/wtmp record to create for a service.
/// Corresponds to systemd's `UtmpMode=` setting.
#[derive(
    Clone, Copy, Eq, PartialEq, Hash, Debug, serde::Serialize, serde::Deserialize, Default,
)]
pub enum UtmpMode {
    /// Write an INIT_PROCESS record (default).
    #[default]
    Init,
    /// Write a LOGIN_PROCESS record (for getty-like services).
    Login,
    /// Write a USER_PROCESS record.
    User,
}

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub enum ServiceType {
    Simple,
    Notify,
    /// Like Notify, but the service also supports reloading via SIGHUP.
    /// At startup this behaves identically to Notify (waits for READY=1).
    NotifyReload,
    Dbus,
    OneShot,
    /// The started process is expected to fork and exit. The parent's exit
    /// signals successful startup. If `PIDFile=` is set the daemon PID is
    /// read from that file; otherwise the service is tracked without a
    /// main PID.
    Forking,
    /// Behaves like Simple, but the service manager delays starting the
    /// service until all active jobs are dispatched (with a 5-second
    /// timeout). This is primarily used for improving console output
    /// ordering and has no effect on service dependencies.
    Idle,
    /// Like Simple, but the service manager waits for the exec() call to
    /// succeed before considering the service started. If the binary
    /// cannot be found or fails to execute, the service enters a failed
    /// state immediately rather than appearing active. This is the
    /// recommended type for long-running services. See systemd.service(5).
    Exec,
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum NotifyKind {
    Main,
    Exec,
    All,
    None,
}

#[derive(Clone, Copy, Eq, PartialEq, Debug, Default)]
pub enum KillMode {
    /// Kill all processes in the control group (default)
    #[default]
    ControlGroup,
    /// Only kill the main process
    Process,
    /// Send SIGTERM to main process, SIGKILL to remaining processes in the control group
    Mixed,
    /// No processes are killed, only ExecStop commands are run
    None,
}

/// Whether to delegate cgroup control to the service process
#[derive(Clone, Eq, PartialEq, Debug, Default)]
pub enum Delegate {
    /// No delegation (default)
    #[default]
    No,
    /// Delegate all supported controllers
    Yes,
    /// Delegate specific controllers
    Controllers(Vec<String>),
}

/// MemoryPressureWatch= — configures whether to watch for memory pressure
/// events via PSI (Pressure Stall Information). Parsed and stored; no runtime
/// enforcement (requires cgroup + PSI support). See systemd.resource-control(5).
#[derive(Clone, Eq, PartialEq, Debug, serde::Serialize, serde::Deserialize, Default)]
pub enum MemoryPressureWatch {
    /// Automatically enable if the service has a dedicated cgroup and PSI is
    /// available (default).
    #[default]
    Auto,
    /// Always watch for memory pressure.
    On,
    /// Never watch for memory pressure.
    Off,
    /// Do not set the MEMORY_PRESSURE_WATCH environment variable at all.
    Skip,
}

/// KeyringMode= — controls how the kernel session keyring is set up for the
/// service. See session-keyring(7) and systemd.exec(5).
#[derive(Clone, Eq, PartialEq, Debug, serde::Serialize, serde::Deserialize, Default)]
pub enum KeyringMode {
    /// No special keyring setup; kernel default behaviour applies.
    Inherit,
    /// A new session keyring is allocated and not linked to the user keyring.
    /// Recommended for system services so that multiple services under the
    /// same UID do not share key material (default for system services).
    #[default]
    Private,
    /// A new session keyring is allocated and the user keyring of the
    /// configured User= is linked into it, allowing key sharing between
    /// units running under the same user.
    Shared,
}

/// ProtectSystem= — controls whether the service has read-only access to the
/// OS file system hierarchy. See systemd.exec(5).
#[derive(
    Clone, Copy, Eq, PartialEq, Hash, Debug, serde::Serialize, serde::Deserialize, Default,
)]
pub enum ProtectSystem {
    /// No file system protection (default).
    #[default]
    No,
    /// Mount /usr and the boot loader directories (/boot, /efi) read-only.
    Yes,
    /// Like `Yes`, but additionally mount /etc read-only.
    Full,
    /// Mount the entire file system hierarchy read-only, except for /dev,
    /// /proc, /sys, and API mount points. Implies `ReadWritePaths=`,
    /// `ReadOnlyPaths=`, `InaccessiblePaths=` are still honoured.
    Strict,
}

/// ProtectHome= — controls whether /home, /root, and /run/user are
/// accessible to the service. See systemd.exec(5).
#[derive(
    Clone, Copy, Eq, PartialEq, Hash, Debug, serde::Serialize, serde::Deserialize, Default,
)]
pub enum ProtectHome {
    /// No protection (default). Directories are left accessible as normal.
    #[default]
    No,
    /// Make /home, /root, and /run/user inaccessible and empty for the
    /// service's processes.
    Yes,
    /// Make /home, /root, and /run/user read-only for the service's
    /// processes.
    ReadOnly,
    /// Mount a tmpfs file system on /home, /root, and /run/user for the
    /// service's processes. Files or directories below these may be
    /// created but are not visible to other processes.
    Tmpfs,
}

/// ProtectProc= — controls the `hidepid=` mount option of the procfs instance
/// for the unit. Controls which `/proc/PID` directories are visible and
/// accessible. See systemd.exec(5).
#[derive(
    Clone, Copy, Eq, PartialEq, Hash, Debug, serde::Serialize, serde::Deserialize, Default,
)]
pub enum ProtectProc {
    /// No restrictions on /proc/ access or visibility (default).
    #[default]
    Default,
    /// Most of other users' process metadata in /proc/ is taken away.
    Noaccess,
    /// Processes owned by other users are hidden from /proc/.
    Invisible,
    /// All processes that cannot be ptrace()'d are hidden.
    Ptraceable,
}

/// ProcSubset= — controls which subset of /proc/ is mounted for the unit.
/// Matches systemd's `ProcSubset=` exec setting.
/// See <https://www.freedesktop.org/software/systemd/man/systemd.exec.html#ProcSubset=>.
#[derive(
    Clone, Copy, Eq, PartialEq, Hash, Debug, serde::Serialize, serde::Deserialize, Default,
)]
pub enum ProcSubset {
    /// The full /proc/ file system is mounted (default).
    #[default]
    All,
    /// Only the /proc/$PID process-specific subdirectories are accessible;
    /// system-wide directories such as /proc/sys/, /proc/sysvipc/, etc.
    /// are not available.
    Pid,
}

/// DeferTrigger= — controls whether to defer triggering the associated service
/// when a connection comes in. May only be used when Accept=no.
/// See systemd.socket(5).
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum DeferTrigger {
    /// Normal triggering behavior (default).
    No,
    /// Use lenient job mode; wait for job queue to complete; fail if
    /// conflict remains after jobs finish or timeout.
    Yes,
    /// Like Yes, but always wait until DeferTriggerMaxSec= elapses
    /// before giving up.
    Patient,
}

/// Timestamping= — controls the SO_TIMESTAMP or SO_TIMESTAMPNS socket
/// options, enabling timestamping metadata on ingress network traffic.
/// See systemd.socket(5).
#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub enum Timestamping {
    /// No timestamping (default).
    Off,
    /// Microsecond-precision timestamping (SO_TIMESTAMP).
    Microseconds,
    /// Nanosecond-precision timestamping (SO_TIMESTAMPNS).
    Nanoseconds,
}

/// RuntimeDirectoryPreserve= — controls whether runtime directories created
/// by `RuntimeDirectory=` are removed when the service stops. See systemd.exec(5).
#[derive(
    Clone, Copy, Eq, PartialEq, Hash, Debug, serde::Serialize, serde::Deserialize, Default,
)]
pub enum RuntimeDirectoryPreserve {
    /// Always remove runtime directories when the service is stopped (default).
    #[default]
    No,
    /// Always preserve runtime directories when the service is stopped.
    Yes,
    /// Preserve runtime directories across service restarts, but remove them
    /// when the service is fully stopped.
    Restart,
}

/// FileDescriptorStorePreserve= — controls whether file descriptors stored
/// in the service manager (via FDSTORE=1 sd_notify messages) are preserved
/// across service restarts or stops. See systemd.service(5).
#[derive(
    Clone, Copy, Eq, PartialEq, Hash, Debug, serde::Serialize, serde::Deserialize, Default,
)]
pub enum FileDescriptorStorePreserve {
    /// Discard stored file descriptors when the service is stopped (default).
    #[default]
    No,
    /// Preserve stored file descriptors as long as the unit exists in memory,
    /// even if the service is stopped.
    Yes,
    /// Preserve stored file descriptors across service restarts, but discard
    /// them when the service is fully stopped.
    Restart,
}

/// RestrictNamespaces= — restricts access to Linux namespace types for the
/// service. See systemd.exec(5).
#[derive(Clone, Eq, PartialEq, Debug, serde::Serialize, serde::Deserialize, Default)]
pub enum RestrictNamespaces {
    /// No namespace restrictions (default).
    #[default]
    No,
    /// Restrict all namespace creation and joining.
    Yes,
    /// Restrict to only the listed namespace types (allow-list).
    Allow(Vec<String>),
    /// Allow all namespace types except the listed ones (deny-list, ~ prefix).
    Deny(Vec<String>),
}

/// Limit on the number of tasks (processes/threads) in the service's cgroup
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum TasksMax {
    /// Absolute limit on number of tasks
    Value(u64),
    /// Percentage of the system's overall task limit
    Percent(u64),
    /// No limit
    Infinity,
}

/// A memory resource limit value used by MemoryMin=, MemoryLow=, MemoryHigh=,
/// MemoryMax=, MemorySwapMax=, etc. Accepts an absolute byte value (with
/// optional K, M, G, T, P, E suffixes using base 1024), a percentage of
/// physical memory, or "infinity". See systemd.resource-control(5).
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum MemoryLimit {
    /// Absolute byte value
    Bytes(u64),
    /// Percentage of physical memory (0–100)
    Percent(u64),
    /// No limit
    Infinity,
}

/// CPUWeight= / StartupCPUWeight= — CPU weight for the unit's cgroup under
/// the CPU controller. Range 1–10000, default 100. See systemd.resource-control(5).
pub type CpuWeight = u64;

/// CPUQuota= — CPU time quota as a percentage. 100% means one full CPU core.
/// Values above 100% are allowed for multi-core allocation (e.g. 200% = 2 cores).
/// See systemd.resource-control(5).
pub type CpuQuota = u64;

/// IOWeight= / StartupIOWeight= — I/O weight for the unit's cgroup under
/// the IO controller. Range 1–10000, default 100. See systemd.resource-control(5).
pub type IoWeight = u64;

/// A per-device I/O limit used by IODeviceWeight=, IOReadBandwidthMax=,
/// IOWriteBandwidthMax=, IOReadIOPSMax=, IOWriteIOPSMax=.
/// Format: "/dev/path value" where value is a weight (1–10000), byte rate,
/// or IOPS count. See systemd.resource-control(5).
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct IoDeviceLimit {
    /// Device node path (e.g. "/dev/sda")
    pub device: String,
    /// Limit value — interpretation depends on which directive this is from:
    /// - IODeviceWeight=: weight 1–10000
    /// - IOReadBandwidthMax=/IOWriteBandwidthMax=: bytes per second
    /// - IOReadIOPSMax=/IOWriteIOPSMax=: IOPS count
    pub value: u64,
}

/// Parse a CPUWeight= or StartupCPUWeight= value.
/// Accepts "idle" (mapped to 1), plain integers 1–10000, or empty (None).
pub fn parse_cpu_weight(val: &str) -> Result<Option<CpuWeight>, String> {
    let val = val.trim();
    if val.is_empty() {
        return Ok(None);
    }
    if val.eq_ignore_ascii_case("idle") {
        return Ok(Some(1));
    }
    let weight = val
        .parse::<u64>()
        .map_err(|_| format!("CPUWeight is not a valid number: {val}"))?;
    if !(1..=10000).contains(&weight) {
        return Err(format!("CPUWeight={val} is out of range (1–10000)"));
    }
    Ok(Some(weight))
}

/// Parse a CPUQuota= value. Accepts a percentage (e.g. "20%", "200%") or empty.
/// Returns the percentage as a plain integer (20 means 20%, 200 means 200%).
pub fn parse_cpu_quota(val: &str) -> Result<Option<CpuQuota>, String> {
    let val = val.trim();
    if val.is_empty() {
        return Ok(None);
    }
    let pct_str = val
        .strip_suffix('%')
        .ok_or_else(|| format!("CPUQuota must end with '%': {val}"))?;
    let pct = pct_str
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("CPUQuota is not a valid percentage: {val}"))?;
    if pct == 0 {
        return Err("CPUQuota=0% is not valid".to_string());
    }
    Ok(Some(pct))
}

/// Parse an IOWeight= or StartupIOWeight= value.
/// Accepts plain integers 1–10000 or empty (None).
pub fn parse_io_weight(val: &str) -> Result<Option<IoWeight>, String> {
    let val = val.trim();
    if val.is_empty() {
        return Ok(None);
    }
    let weight = val
        .parse::<u64>()
        .map_err(|_| format!("IOWeight is not a valid number: {val}"))?;
    if !(1..=10000).contains(&weight) {
        return Err(format!("IOWeight={val} is out of range (1–10000)"));
    }
    Ok(Some(weight))
}

/// Parse a per-device I/O limit line: "DEVICE_PATH VALUE" where VALUE is
/// either a plain number or a byte-size with suffix. Used by IODeviceWeight=,
/// IOReadBandwidthMax=, IOWriteBandwidthMax=, IOReadIOPSMax=, IOWriteIOPSMax=.
pub fn parse_io_device_limit(val: &str) -> Result<Option<IoDeviceLimit>, String> {
    let val = val.trim();
    if val.is_empty() {
        return Ok(None);
    }
    let (device, value_str) = val
        .split_once(char::is_whitespace)
        .ok_or_else(|| format!("IODevice limit must be 'DEVICE VALUE': {val}"))?;
    let value_str = value_str.trim();
    // Try SI byte-size parsing first (handles K/M/G suffixes with base 1000,
    // matching upstream systemd's IO limit parsing), fall back to plain u64
    let value = parse_byte_size_si(value_str).or_else(|_| {
        value_str
            .parse::<u64>()
            .map_err(|_| format!("IODevice limit value is not valid: {value_str}"))
    })?;
    Ok(Some(IoDeviceLimit {
        device: device.to_owned(),
        value,
    }))
}

/// Parse a systemd byte value with optional K, M, G, T, P, E suffix (base 1024).
/// Also accepts percentage values (e.g. "30%") and "infinity".
/// Returns `None` for empty or unrecognised strings.
pub fn parse_memory_limit(val: &str) -> Result<Option<MemoryLimit>, String> {
    let val = val.trim();
    if val.is_empty() {
        return Ok(None);
    }
    if val.eq_ignore_ascii_case("infinity") {
        return Ok(Some(MemoryLimit::Infinity));
    }
    if let Some(pct) = val.strip_suffix('%') {
        let pct_val = pct
            .trim()
            .parse::<u64>()
            .map_err(|_| format!("memory limit percentage is not a valid number: {val}"))?;
        return Ok(Some(MemoryLimit::Percent(pct_val)));
    }
    // Delegate byte-value parsing (with optional K/M/G/T/P/E suffix) to parse_byte_size
    let bytes = parse_byte_size(val)
        .map_err(|_| format!("memory limit is not a valid byte value: {val}"))?;
    Ok(Some(MemoryLimit::Bytes(bytes)))
}

/// Parse a systemd byte-size value with optional K, M, G, T, P, E suffix.
/// Returns the value in bytes as a `u64`.
///
/// Accepts plain integers (e.g. "8388608") as well as suffixed values
/// (e.g. "128K", "128M", "1G"). The suffix is case-insensitive.
/// Returns an error for empty, non-numeric, or otherwise invalid strings.
///
/// The `base` parameter controls the multiplier: 1024 for memory/binary
/// (KiB, MiB, …), 1000 for IO bandwidth/SI (KB, MB, …).
fn parse_byte_size_base(val: &str, base: u64) -> Result<u64, String> {
    let val = val.trim();
    if val.is_empty() {
        return Err("byte size value is empty".to_owned());
    }
    let last = val.as_bytes()[val.len() - 1];
    let (num_str, multiplier): (&str, u64) = match last | 0x20 {
        // case-insensitive
        b'k' => (&val[..val.len() - 1], base),
        b'm' => (&val[..val.len() - 1], base * base),
        b'g' => (&val[..val.len() - 1], base * base * base),
        b't' => (&val[..val.len() - 1], base * base * base * base),
        b'p' => (&val[..val.len() - 1], base * base * base * base * base),
        b'e' => (
            &val[..val.len() - 1],
            base * base * base * base * base * base,
        ),
        _ => (val, 1),
    };
    let num = num_str
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("not a valid byte size: {val}"))?;
    num.checked_mul(multiplier)
        .ok_or_else(|| format!("byte size overflows u64: {val}"))
}

/// Parse a byte-size value with binary (base-1024) suffixes for memory limits.
pub fn parse_byte_size(val: &str) -> Result<u64, String> {
    parse_byte_size_base(val, 1024)
}

/// Parse a byte-size value with SI (base-1000) suffixes for IO bandwidth/IOPS.
/// Matches upstream systemd's `parse_size(rvalue, 1000, &num)`.
pub fn parse_byte_size_si(val: &str) -> Result<u64, String> {
    parse_byte_size_base(val, 1000)
}

/// A single rlimit value: either a numeric value or infinity
#[derive(Clone, Copy, Eq, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub enum RLimitValue {
    /// A specific numeric value
    Value(u64),
    /// RLIM_INFINITY — no limit
    Infinity,
}

/// A resource limit with soft and hard values (as used by setrlimit)
#[derive(Clone, Copy, Eq, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct ResourceLimit {
    pub soft: RLimitValue,
    pub hard: RLimitValue,
}

/// Parse a resource limit value string like "10000", "10000:16384", or "infinity".
/// Returns `None` if the value is empty or cannot be parsed.
/// Parse a single rlimit value string with optional byte suffix (K/M/G/T).
fn parse_rlimit_value_with_suffix(s: &str) -> Option<RLimitValue> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("infinity") {
        return Some(RLimitValue::Infinity);
    }
    let (num_str, multiplier) =
        if let Some(prefix) = s.strip_suffix('M').or_else(|| s.strip_suffix('m')) {
            (prefix, 1024u64 * 1024)
        } else if let Some(prefix) = s.strip_suffix('G').or_else(|| s.strip_suffix('g')) {
            (prefix, 1024u64 * 1024 * 1024)
        } else if let Some(prefix) = s.strip_suffix('T').or_else(|| s.strip_suffix('t')) {
            (prefix, 1024u64 * 1024 * 1024 * 1024)
        } else if let Some(prefix) = s.strip_suffix('K').or_else(|| s.strip_suffix('k')) {
            (prefix, 1024u64)
        } else {
            (s, 1u64)
        };
    let num = num_str.parse::<u64>().ok()?;
    Some(RLimitValue::Value(num.saturating_mul(multiplier)))
}

pub fn parse_resource_limit(val: &str) -> Option<ResourceLimit> {
    let val = val.trim();
    if val.is_empty() {
        return None;
    }
    if val.eq_ignore_ascii_case("infinity") {
        return Some(ResourceLimit {
            soft: RLimitValue::Infinity,
            hard: RLimitValue::Infinity,
        });
    }
    if let Some((soft_str, hard_str)) = val.split_once(':') {
        let soft = parse_rlimit_value_with_suffix(soft_str)?;
        let hard = parse_rlimit_value_with_suffix(hard_str)?;
        Some(ResourceLimit { soft, hard })
    } else {
        let v = parse_rlimit_value_with_suffix(val)?;
        Some(ResourceLimit { soft: v, hard: v })
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub enum ServiceRestart {
    /// Never restart the service automatically.
    No,
    /// Always restart the service when it exits, regardless of exit status.
    Always,
    /// Restart only if the service exited cleanly (exit code 0 or one of the
    /// "clean" signals: SIGHUP, SIGINT, SIGTERM, SIGPIPE).
    OnSuccess,
    /// Restart only if the service exited with a non-zero exit code, was
    /// terminated by a signal (other than the "clean" ones), timed out, or
    /// hit a watchdog timeout.
    OnFailure,
    /// Restart only if the service was terminated by a signal, timed out, or
    /// hit a watchdog timeout (i.e. not on clean or unclean exit codes).
    OnAbnormal,
    /// Restart only if the service was terminated by an uncaught signal
    /// (not SIGHUP, SIGINT, SIGTERM, SIGPIPE).
    OnAbort,
    /// Restart only if the watchdog timeout for the service expired.
    /// Restarts the service when the watchdog timeout fires (i.e. the service
    /// failed to send `WATCHDOG=1` within the configured `WatchdogSec=`
    /// interval). Enforced by the background watchdog thread.
    OnWatchdog,
}

/// Configures when to consider a service as exited / dead.
/// See `systemd.service(5)` `ExitType=`.
#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug, Default)]
pub enum ExitType {
    /// The service is considered dead when the main process exits (default).
    #[default]
    Main,
    /// The service is considered dead only when the last process in its
    /// cgroup exits.
    Cgroup,
}

/// Configures the Out-Of-Memory (OOM) killer policy for a service.
/// See `systemd.service(5)` `OOMPolicy=`.
#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug, Default)]
pub enum OOMPolicy {
    /// The kernel or `systemd-oomd` kills the process, but the service
    /// manager does not take further action (default for non-root slices).
    Continue,
    /// When an OOM kill happens, the service manager terminates the
    /// remaining processes of the service and puts it into a failed state.
    #[default]
    Stop,
    /// Like `Stop`, but the service is also killed immediately (SIGKILL).
    Kill,
}

/// Configures how a service is restarted.
/// See `systemd.service(5)` `RestartMode=`.
#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug, Default)]
pub enum RestartMode {
    /// Restart without going through the full deactivation path.
    /// OnFailure= units are not triggered and dependent units (BindsTo=,
    /// required_by) are not stopped during the restart.
    #[default]
    Direct,
    /// Go through the full deactivation path on restart. OnFailure= units
    /// are triggered and dependent units (BindsTo=, required_by) are stopped
    /// then re-activated after the service restarts.
    Normal,
}

/// Configures the action to take when a start or stop timeout is reached.
/// See `systemd.service(5)` `TimeoutStartFailureMode=` / `TimeoutStopFailureMode=`.
#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug, Default)]
pub enum TimeoutFailureMode {
    /// Send the configured stop signal (KillSignal=), then SIGKILL after
    /// the final timeout. This is the default.
    #[default]
    Terminate,
    /// Immediately send SIGABRT, useful for triggering a core dump for
    /// debugging.
    Abort,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub enum Timeout {
    Duration(std::time::Duration),
    Infinity,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub enum StdIoOption {
    /// StandardOutput/StandardError connected to /dev/null
    Null,
    /// Inherit from the service manager (or from stdin for stdout/stderr)
    Inherit,
    /// Log to the journal (not yet implemented, treated as inherit)
    Journal,
    /// Log to /dev/kmsg (not yet implemented, treated as inherit)
    Kmsg,
    /// Connect to the TTY device (from TTYPath=, default /dev/console).
    /// When StandardInput is also tty, stdout/stderr share the same TTY fd.
    /// When StandardInput is NOT tty, the TTY is opened independently for output.
    /// Matches systemd's `StandardOutput=tty` / `StandardError=tty`.
    Tty,
    /// Write to a specific file
    File(PathBuf),
    /// Append to a specific file
    AppendFile(PathBuf),
    /// Connect to the socket fd (for Accept=yes socket-activated services).
    /// The socket fd (fd 3, the first LISTEN_FD) is dup'd to stdout/stderr.
    Socket,
}

/// How stdin should be set up for the service process.
/// Matches systemd's StandardInput= setting.
#[derive(Clone, Eq, PartialEq, Debug, serde::Serialize, serde::Deserialize, Default)]
pub enum StandardInput {
    /// stdin is connected to /dev/null (default)
    #[default]
    Null,
    /// stdin is connected to a TTY (from TTYPath=, default /dev/console)
    Tty,
    /// Like Tty, but force-acquire the TTY even if another process owns it
    TtyForce,
    /// Like Tty, but fail if the TTY cannot be opened exclusively
    TtyFail,
    /// stdin is connected to the socket fd (for Accept=yes socket-activated
    /// services). The accepted connection fd (fd 3, the first LISTEN_FD) is
    /// dup'd to stdin.
    Socket,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub enum CommandlinePrefix {
    AtSign,
    Minus,
    Colon,
    Plus,
    Exclamation,
    DoubleExclamation,
    /// `|` prefix: run the command via the user's login shell (`shell -el -c "cmd args..."`)
    Pipe,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct Commandline {
    pub cmd: String,
    pub args: Vec<String>,
    pub prefixes: Vec<CommandlinePrefix>,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct EnvVars {
    pub vars: Vec<(String, String)>,
}

impl std::fmt::Display for Commandline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for p in &self.prefixes {
            match p {
                CommandlinePrefix::Minus => write!(f, "-")?,
                CommandlinePrefix::AtSign => write!(f, "@")?,
                CommandlinePrefix::Plus => write!(f, "+")?,
                CommandlinePrefix::Colon => write!(f, ":")?,
                CommandlinePrefix::Exclamation => write!(f, "!")?,
                CommandlinePrefix::DoubleExclamation => write!(f, "!!")?,
                CommandlinePrefix::Pipe => write!(f, "|")?,
            }
        }
        write!(f, "{}", self.cmd)?;
        for arg in &self.args {
            write!(f, " {arg}")?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct ParsingError {
    inner: ParsingErrorReason,
    path: std::path::PathBuf,
}

impl ParsingError {
    #[must_use]
    pub const fn new(reason: ParsingErrorReason, path: std::path::PathBuf) -> Self {
        Self {
            inner: reason,
            path,
        }
    }
}

#[derive(Debug)]
pub enum ParsingErrorReason {
    UnknownSetting(String, String),
    UnusedSetting(String),
    UnsupportedSetting(String),
    MissingSetting(String),
    SettingTooManyValues(String, Vec<String>),
    SectionTooOften(String),
    SectionNotFound(String),
    UnknownSection(String),
    UnknownSocketAddr(String),
    FileError(Box<dyn std::error::Error>),
    Generic(String),
}

impl std::fmt::Display for ParsingError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match &self.inner {
            ParsingErrorReason::UnknownSetting(name, value) => {
                write!(
                    f,
                    "In file {:?}: setting {} was set to unrecognized value: {}",
                    self.path, name, value
                )?;
            }
            ParsingErrorReason::UnusedSetting(name) => {
                write!(
                    f,
                    "In file {:?}: unused setting {} occurred",
                    self.path, name
                )?;
            }
            ParsingErrorReason::MissingSetting(name) => {
                write!(
                    f,
                    "In file {:?}: required setting {} missing",
                    self.path, name
                )?;
            }
            ParsingErrorReason::SectionNotFound(name) => {
                write!(
                    f,
                    "In file {:?}: Section {} wasn't found but is required",
                    self.path, name
                )?;
            }
            ParsingErrorReason::UnknownSection(name) => {
                write!(f, "In file {:?}: Section {} is unknown", self.path, name)?;
            }
            ParsingErrorReason::SectionTooOften(name) => {
                write!(
                    f,
                    "In file {:?}: section {} occurred multiple times",
                    self.path, name
                )?;
            }
            ParsingErrorReason::UnknownSocketAddr(addr) => {
                write!(
                    f,
                    "In file {:?}: Can not open sockets of addr: {}",
                    self.path, addr
                )?;
            }
            ParsingErrorReason::UnsupportedSetting(addr) => {
                write!(
                    f,
                    "In file {:?}: Setting not supported by this build (maybe need to enable feature flag?): {}",
                    self.path, addr
                )?;
            }
            ParsingErrorReason::SettingTooManyValues(name, values) => {
                write!(
                    f,
                    "In file {:?}: setting {} occurred with too many values: {:?}",
                    self.path, name, values
                )?;
            }
            ParsingErrorReason::FileError(e) => {
                write!(f, "While parsing file {:?}: {}", self.path, e)?;
            }
            ParsingErrorReason::Generic(e) => {
                write!(f, "While parsing file {:?}: {}", self.path, e)?;
            }
        }

        Ok(())
    }
}

// This is important for other errors to wrap this one.
impl std::error::Error for ParsingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // Generic error, underlying cause isn't tracked.
        if let ParsingErrorReason::FileError(err) = &self.inner {
            Some(err.as_ref())
        } else {
            None
        }
    }
}

impl std::convert::From<Box<std::io::Error>> for ParsingErrorReason {
    fn from(err: Box<std::io::Error>) -> Self {
        Self::FileError(err)
    }
}

/// Test helper module that re-exports private functions for unit tests.
#[cfg(test)]
pub mod mod_tests_helper {
    pub fn test_looks_like_machine_id(s: &str) -> bool {
        super::looks_like_machine_id(s)
    }

    pub fn test_parse_cpu_range_count(s: &str) -> Option<u32> {
        super::parse_cpu_range_count(s)
    }

    pub fn test_parse_memory_condition(value: &str) -> Option<u64> {
        super::parse_memory_condition(value)
    }
}

//! systemd-sysext — Activate/deactivate system extension images.
//!
//! A drop-in replacement for `systemd-sysext(8)`. This tool manages system
//! extension images that overlay additional files on top of `/usr/` and
//! `/opt/` using read-only overlayfs mounts.
//!
//! System extension images are directories, disk images, or DDIs located in:
//!   - `/etc/extensions/`
//!   - `/run/extensions/`
//!   - `/var/lib/extensions/`
//!   - `/usr/lib/extensions/`
//!   - `/usr/local/lib/extensions/`
//!
//! Each extension image must contain an `extension-release.<NAME>` file in
//! `usr/lib/extension-release.d/` that identifies the extension and declares
//! compatibility with the host OS (via `ID=`, `VERSION_ID=`, `SYSEXT_LEVEL=`,
//! `ARCHITECTURE=`, etc.).
//!
//! Subcommands:
//!
//!   systemd-sysext status
//!       Show current extension status and hierarchy info.
//!
//!   systemd-sysext list
//!       List all discovered extension images.
//!
//!   systemd-sysext merge
//!       Activate extensions by creating overlayfs mounts over /usr/ and /opt/.
//!
//!   systemd-sysext unmerge
//!       Deactivate extensions by unmounting the overlayfs layers.
//!
//!   systemd-sysext refresh
//!       Unmerge then re-merge (equivalent to unmerge + merge).
//!
//!   systemd-sysext check-inhibit
//!       Check whether merge is currently inhibited.
//!
//! Options:
//!   --root=PATH          Operate on an alternate root filesystem
//!   --force              Force merge even if already merged
//!   --no-reload          Don't reload the service manager after merge/unmerge
//!   --no-pager           Don't pipe output through a pager
//!   --json=MODE          Output in JSON format (short, pretty, off)
//!   -h --help            Show help
//!      --version         Show version
//!
//! Exit codes:
//!   0 — success
//!   1 — error

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Directories to search for extension images, in priority order.
const EXTENSION_DIRS: &[&str] = &[
    "/etc/extensions",
    "/run/extensions",
    "/var/lib/extensions",
    "/usr/lib/extensions",
    "/usr/local/lib/extensions",
];

/// Hierarchies that sysext manages (overlayfs mount points).
const DEFAULT_HIERARCHIES: &[&str] = &["/usr", "/opt"];

/// Where we store the workspace directories for overlayfs.
const SYSEXT_WORK_DIR: &str = "/run/systemd/sysext";

/// Marker file indicating extensions are currently merged.
const SYSEXT_MARKER: &str = "/run/systemd/sysext/extensions.merged";

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// A discovered system extension image.
#[derive(Debug, Clone)]
struct Extension {
    /// Name of the extension (derived from directory/image name).
    name: String,
    /// Path to the extension image/directory.
    path: PathBuf,
    /// Type of the extension source.
    ext_type: ExtensionType,
    /// Parsed extension-release metadata.
    release: HashMap<String, String>,
    /// Whether this extension is compatible with the host.
    compatible: bool,
    /// Hierarchies this extension provides content for.
    hierarchies: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtensionType {
    /// A plain directory containing the extension tree.
    Directory,
    /// A raw disk image (not yet supported, detected only).
    RawImage,
}

impl std::fmt::Display for ExtensionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtensionType::Directory => write!(f, "directory"),
            ExtensionType::RawImage => write!(f, "raw"),
        }
    }
}

/// Host OS identity for compatibility checking.
#[derive(Debug, Clone, Default)]
struct HostInfo {
    id: Option<String>,
    version_id: Option<String>,
    sysext_level: Option<String>,
    architecture: Option<String>,
}

/// Parsed command-line arguments.
#[derive(Debug, Clone)]
struct Args {
    command: Command,
    root: PathBuf,
    force: bool,
    no_reload: bool,
    json_mode: JsonMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Status,
    List,
    Merge,
    Unmerge,
    Refresh,
    CheckInhibit,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonMode {
    Off,
    Short,
    Pretty,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            command: Command::Status,
            root: PathBuf::from("/"),
            force: false,
            no_reload: false,
            json_mode: JsonMode::Off,
        }
    }
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut positional_set = false;

    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];

        let (key, value) = if let Some(pos) = arg.find('=') {
            (&arg[..pos], Some(arg[pos + 1..].to_string()))
        } else {
            (arg.as_str(), None)
        };

        match key {
            "--help" | "-h" => {
                args.command = Command::Help;
                return Ok(args);
            }
            "--version" => {
                println!("systemd-sysext (rust-systemd)");
                process::exit(0);
            }
            "--root" => {
                let v = value_or_next(argv, &mut i, value, "--root")?;
                args.root = PathBuf::from(v);
            }
            "--force" => args.force = true,
            "--no-reload" => args.no_reload = true,
            "--no-pager" => { /* accepted and ignored */ }
            "--json" => {
                let v = value_or_next(argv, &mut i, value, "--json")?;
                args.json_mode = match v.as_str() {
                    "short" => JsonMode::Short,
                    "pretty" => JsonMode::Pretty,
                    "off" => JsonMode::Off,
                    _ => return Err(format!("Unknown JSON mode: {}", v)),
                };
            }
            _ if !arg.starts_with('-') && !positional_set => {
                args.command = match arg.as_str() {
                    "status" => Command::Status,
                    "list" => Command::List,
                    "merge" => Command::Merge,
                    "unmerge" => Command::Unmerge,
                    "refresh" => Command::Refresh,
                    "check-inhibit" => Command::CheckInhibit,
                    other => return Err(format!("Unknown command: {}", other)),
                };
                positional_set = true;
            }
            other => {
                return Err(format!("Unknown option: {}", other));
            }
        }

        i += 1;
    }

    Ok(args)
}

fn value_or_next(
    args: &[String],
    i: &mut usize,
    value: Option<String>,
    name: &str,
) -> Result<String, String> {
    if let Some(v) = value {
        Ok(v)
    } else if *i + 1 < args.len() {
        *i += 1;
        Ok(args[*i].clone())
    } else {
        Err(format!("Option {} requires an argument", name))
    }
}

// ---------------------------------------------------------------------------
// Host info
// ---------------------------------------------------------------------------

fn read_host_info(root: &Path) -> HostInfo {
    let mut info = HostInfo::default();

    let os_release_paths = [root.join("etc/os-release"), root.join("usr/lib/os-release")];

    for path in &os_release_paths {
        if let Ok(content) = fs::read_to_string(path) {
            let vars = parse_env_file(&content);
            if info.id.is_none() {
                info.id = vars.get("ID").cloned();
            }
            if info.version_id.is_none() {
                info.version_id = vars.get("VERSION_ID").cloned();
            }
            if info.sysext_level.is_none() {
                info.sysext_level = vars.get("SYSEXT_LEVEL").cloned();
            }
            break;
        }
    }

    // Detect architecture
    info.architecture = detect_architecture();

    info
}

fn detect_architecture() -> Option<String> {
    // Use uname to detect architecture
    #[cfg(target_os = "linux")]
    {
        let mut utsname: libc::utsname = unsafe { std::mem::zeroed() };
        if unsafe { libc::uname(&mut utsname) } == 0 {
            let machine = unsafe { std::ffi::CStr::from_ptr(utsname.machine.as_ptr()) };
            if let Ok(m) = machine.to_str() {
                return Some(normalize_architecture(m));
            }
        }
    }
    None
}

fn normalize_architecture(arch: &str) -> String {
    match arch {
        "x86_64" | "amd64" => "x86-64".to_string(),
        "aarch64" | "arm64" => "arm64".to_string(),
        "i386" | "i486" | "i586" | "i686" => "x86".to_string(),
        "armv7l" | "armv7" => "arm".to_string(),
        "riscv64" => "riscv64".to_string(),
        "s390x" => "s390x".to_string(),
        "ppc64le" => "ppc64-le".to_string(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Environment file parsing
// ---------------------------------------------------------------------------

fn parse_env_file(content: &str) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(pos) = line.find('=') {
            let key = line[..pos].trim().to_string();
            let mut value = line[pos + 1..].trim().to_string();
            // Strip quotes
            if ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
                && value.len() >= 2
            {
                value = value[1..value.len() - 1].to_string();
            }
            vars.insert(key, value);
        }
    }
    vars
}

// ---------------------------------------------------------------------------
// Extension discovery
// ---------------------------------------------------------------------------

fn discover_extensions(root: &Path) -> Vec<Extension> {
    let mut extensions = Vec::new();
    let mut seen_names: HashMap<String, usize> = HashMap::new();

    for dir_str in EXTENSION_DIRS {
        let dir = root.join(dir_str.trim_start_matches('/'));
        if !dir.is_dir() {
            continue;
        }

        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            let name = match entry.file_name().to_str() {
                Some(n) => n.to_string(),
                None => continue,
            };

            // Skip hidden files
            if name.starts_with('.') {
                continue;
            }

            // Determine extension type
            let ext_type = if path.is_dir() {
                ExtensionType::Directory
            } else if name.ends_with(".raw") || name.ends_with(".img") {
                ExtensionType::RawImage
            } else {
                continue;
            };

            // Strip .raw/.img suffix from name
            let clean_name = name
                .strip_suffix(".raw")
                .or_else(|| name.strip_suffix(".img"))
                .unwrap_or(&name)
                .to_string();

            // Only process the first occurrence (higher priority dirs first)
            if seen_names.contains_key(&clean_name) {
                continue;
            }

            // Try to read extension-release file
            let release = read_extension_release(&path, &clean_name, ext_type);

            // Detect which hierarchies the extension provides
            let hierarchies = detect_hierarchies(&path, ext_type);

            let idx = extensions.len();
            seen_names.insert(clean_name.clone(), idx);

            extensions.push(Extension {
                name: clean_name,
                path,
                ext_type,
                release,
                compatible: false, // Will be checked later
                hierarchies,
            });
        }
    }

    // Sort by name
    extensions.sort_by(|a, b| a.name.cmp(&b.name));
    extensions
}

fn read_extension_release(
    path: &Path,
    name: &str,
    ext_type: ExtensionType,
) -> HashMap<String, String> {
    if ext_type == ExtensionType::RawImage {
        // Raw images need mounting — not supported yet
        return HashMap::new();
    }

    // Look for usr/lib/extension-release.d/extension-release.<NAME>
    let release_path = path
        .join("usr/lib/extension-release.d")
        .join(format!("extension-release.{}", name));

    if let Ok(content) = fs::read_to_string(&release_path) {
        return parse_env_file(&content);
    }

    // Also check for a single extension-release file if there's exactly one
    let release_dir = path.join("usr/lib/extension-release.d");
    if let Ok(entries) = fs::read_dir(&release_dir) {
        let files: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("extension-release."))
            })
            .collect();
        if files.len() == 1
            && let Ok(content) = fs::read_to_string(files[0].path())
        {
            return parse_env_file(&content);
        }
    }

    HashMap::new()
}

fn detect_hierarchies(path: &Path, ext_type: ExtensionType) -> Vec<String> {
    if ext_type == ExtensionType::RawImage {
        return Vec::new();
    }

    let mut hierarchies = Vec::new();

    for hier in DEFAULT_HIERARCHIES {
        let hier_path = path.join(hier.trim_start_matches('/'));
        if hier_path.is_dir() {
            // Check that the directory has some content (not just extension-release.d)
            if let Ok(entries) = fs::read_dir(&hier_path) {
                let has_content = entries.filter_map(|e| e.ok()).any(|e| {
                    e.file_name().to_str().is_some_and(|n| {
                        n != "lib" || {
                            // If it's lib/, check it has more than just extension-release.d
                            let lib_path = path.join(hier.trim_start_matches('/')).join("lib");
                            if let Ok(lib_entries) = fs::read_dir(&lib_path) {
                                lib_entries.filter_map(|e| e.ok()).any(|e| {
                                    e.file_name()
                                        .to_str()
                                        .is_some_and(|n| n != "extension-release.d")
                                })
                            } else {
                                false
                            }
                        }
                    })
                });
                if has_content {
                    hierarchies.push(hier.to_string());
                } else {
                    // Even if the only content is extension-release.d, still count it
                    hierarchies.push(hier.to_string());
                }
            }
        }
    }

    hierarchies
}

// ---------------------------------------------------------------------------
// Compatibility checking
// ---------------------------------------------------------------------------

fn check_compatibility(extensions: &mut [Extension], host: &HostInfo) {
    for ext in extensions.iter_mut() {
        ext.compatible = is_compatible(ext, host);
    }
}

fn is_compatible(ext: &Extension, host: &HostInfo) -> bool {
    if ext.release.is_empty() {
        // No extension-release file — not compatible
        return false;
    }

    // Check ID matches
    if let Some(ext_id) = ext.release.get("ID")
        && let Some(host_id) = &host.id
        && ext_id != host_id
        && ext_id != "_any"
    {
        return false;
    }

    // Check VERSION_ID matches (if SYSEXT_LEVEL is not set)
    if !ext.release.contains_key("SYSEXT_LEVEL")
        && let Some(ext_version) = ext.release.get("VERSION_ID")
        && let Some(host_version) = &host.version_id
        && ext_version != host_version
    {
        return false;
    }

    // Check SYSEXT_LEVEL matches
    if let Some(ext_level) = ext.release.get("SYSEXT_LEVEL")
        && let Some(host_level) = &host.sysext_level
        && ext_level != host_level
    {
        return false;
    }

    // Check ARCHITECTURE matches
    if let Some(ext_arch) = ext.release.get("ARCHITECTURE")
        && ext_arch != "_any"
        && let Some(host_arch) = &host.architecture
        && ext_arch != host_arch
    {
        return false;
    }

    // Check SYSEXT_SCOPE (if present, must include "system")
    if let Some(scope) = ext.release.get("SYSEXT_SCOPE") {
        let scopes: Vec<&str> = scope.split_whitespace().collect();
        if !scopes.contains(&"system") {
            return false;
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Merge state
// ---------------------------------------------------------------------------

fn is_merged(root: &Path) -> bool {
    let marker = root.join(SYSEXT_MARKER.trim_start_matches('/'));
    marker.exists()
}

fn write_merge_marker(root: &Path, extensions: &[Extension]) -> io::Result<()> {
    let marker_path = root.join(SYSEXT_MARKER.trim_start_matches('/'));
    if let Some(parent) = marker_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut content = String::new();
    content.push_str("# systemd-sysext merge marker\n");
    content.push_str("# Do not edit — managed by systemd-sysext\n");
    for ext in extensions {
        content.push_str(&format!("{}={}\n", ext.name, ext.path.display()));
    }

    fs::write(&marker_path, content)?;
    Ok(())
}

fn remove_merge_marker(root: &Path) -> io::Result<()> {
    let marker_path = root.join(SYSEXT_MARKER.trim_start_matches('/'));
    if marker_path.exists() {
        fs::remove_file(&marker_path)?;
    }
    Ok(())
}

fn read_merged_extensions(root: &Path) -> Vec<(String, String)> {
    let marker_path = root.join(SYSEXT_MARKER.trim_start_matches('/'));
    let mut extensions = Vec::new();
    if let Ok(content) = fs::read_to_string(&marker_path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(pos) = line.find('=') {
                let name = line[..pos].to_string();
                let path = line[pos + 1..].to_string();
                extensions.push((name, path));
            }
        }
    }
    extensions
}

// ---------------------------------------------------------------------------
// Check inhibit
// ---------------------------------------------------------------------------

fn check_inhibit(root: &Path) -> bool {
    // Check for inhibit file
    let inhibit_path = root.join("run/systemd/sysext/inhibit");
    inhibit_path.exists()
}

// ---------------------------------------------------------------------------
// Overlayfs mount/unmount
// ---------------------------------------------------------------------------

fn do_merge(root: &Path, extensions: &[Extension], force: bool) -> Result<(), String> {
    if is_merged(root) && !force {
        return Err("Extensions are already merged. Use --force to re-merge.".to_string());
    }

    if is_merged(root) && force {
        do_unmerge(root)?;
    }

    // Filter to compatible extensions only
    let compat_exts: Vec<&Extension> = extensions.iter().filter(|e| e.compatible).collect();

    if compat_exts.is_empty() {
        eprintln!("No compatible extensions found, nothing to merge.");
        return Ok(());
    }

    // Create work directory
    let work_base = root.join(SYSEXT_WORK_DIR.trim_start_matches('/'));
    fs::create_dir_all(&work_base)
        .map_err(|e| format!("Failed to create work directory: {}", e))?;

    // For each hierarchy, create an overlayfs mount
    for hierarchy in DEFAULT_HIERARCHIES {
        let hier_trimmed = hierarchy.trim_start_matches('/');
        let mount_point = root.join(hier_trimmed);

        if !mount_point.is_dir() {
            continue;
        }

        // Collect lower dirs from extensions that have content for this hierarchy
        let mut lower_dirs: Vec<String> = Vec::new();

        for ext in &compat_exts {
            let ext_hier = ext.path.join(hier_trimmed);
            if ext_hier.is_dir() {
                lower_dirs.push(ext_hier.to_string_lossy().to_string());
            }
        }

        if lower_dirs.is_empty() {
            continue;
        }

        // The current hierarchy is the lowest layer
        lower_dirs.push(mount_point.to_string_lossy().to_string());

        // Create work and upper dirs for overlayfs
        let work_dir = work_base.join(format!("{}.work", hier_trimmed.replace('/', "-")));
        let upper_dir = work_base.join(format!("{}.upper", hier_trimmed.replace('/', "-")));

        fs::create_dir_all(&work_dir).map_err(|e| format!("Failed to create work dir: {}", e))?;
        fs::create_dir_all(&upper_dir).map_err(|e| format!("Failed to create upper dir: {}", e))?;

        // Build overlayfs mount options
        let lower = lower_dirs.join(":");
        let mount_opts = format!(
            "lowerdir={},upperdir={},workdir={}",
            lower,
            upper_dir.display(),
            work_dir.display()
        );

        // Actually mount overlayfs
        let result = mount_overlayfs(&mount_point, &mount_opts);
        match result {
            Ok(()) => {
                eprintln!(
                    "Merged {} extension(s) into {}.",
                    lower_dirs.len() - 1,
                    hierarchy
                );
            }
            Err(e) => {
                eprintln!("Warning: Failed to mount overlayfs on {}: {}", hierarchy, e);
                // Clean up
                let _ = fs::remove_dir_all(&work_dir);
                let _ = fs::remove_dir_all(&upper_dir);
            }
        }
    }

    // Write merge marker
    let merged: Vec<Extension> = compat_exts.into_iter().cloned().collect();
    write_merge_marker(root, &merged)
        .map_err(|e| format!("Failed to write merge marker: {}", e))?;

    Ok(())
}

fn do_unmerge(root: &Path) -> Result<(), String> {
    if !is_merged(root) {
        eprintln!("Extensions are not currently merged.");
        return Ok(());
    }

    // Unmount each hierarchy
    for hierarchy in DEFAULT_HIERARCHIES.iter().rev() {
        let mount_point = root.join(hierarchy.trim_start_matches('/'));
        if !mount_point.is_dir() {
            continue;
        }

        // Check if it's actually an overlayfs mount
        if is_overlayfs_mount(&mount_point) {
            match umount(&mount_point) {
                Ok(()) => {
                    eprintln!("Unmerged {}.", hierarchy);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to unmount {}: {}", hierarchy, e);
                }
            }
        }
    }

    // Clean up work directory
    let work_base = root.join(SYSEXT_WORK_DIR.trim_start_matches('/'));
    if work_base.is_dir() {
        // Remove work and upper dirs but keep the base
        if let Ok(entries) = fs::read_dir(&work_base) {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.ends_with(".work") || name_str.ends_with(".upper") {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
    }

    // Remove merge marker
    remove_merge_marker(root).map_err(|e| format!("Failed to remove merge marker: {}", e))?;

    Ok(())
}

fn mount_overlayfs(target: &Path, options: &str) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let source = std::ffi::CString::new("overlay").unwrap();
        let target_c = std::ffi::CString::new(target.to_string_lossy().as_bytes()).unwrap();
        let fstype = std::ffi::CString::new("overlay").unwrap();
        let opts = std::ffi::CString::new(options).unwrap();

        let ret = unsafe {
            libc::mount(
                source.as_ptr(),
                target_c.as_ptr(),
                fstype.as_ptr(),
                libc::MS_RDONLY,
                opts.as_ptr() as *const libc::c_void,
            )
        };

        if ret != 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = (target, options);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "overlayfs is only supported on Linux",
        ))
    }
}

fn umount(target: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let target_c = std::ffi::CString::new(target.to_string_lossy().as_bytes()).unwrap();
        let ret = unsafe { libc::umount2(target_c.as_ptr(), libc::MNT_DETACH) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = target;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "umount is only supported on Linux",
        ))
    }
}

fn is_overlayfs_mount(path: &Path) -> bool {
    if let Ok(content) = fs::read_to_string("/proc/self/mountinfo") {
        let path_str = path.to_string_lossy();
        for line in content.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            // mountinfo format: id parent major:minor root mount_point ... - fstype source opts
            if fields.len() > 8 {
                // Find the separator "-"
                if let Some(sep_pos) = fields.iter().position(|&f| f == "-")
                    && sep_pos + 1 < fields.len()
                    && fields[sep_pos + 1] == "overlay"
                    && fields.len() > 4
                    && fields[4] == path_str
                {
                    return true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Output: status
// ---------------------------------------------------------------------------

fn cmd_status(root: &Path, extensions: &[Extension], json_mode: JsonMode) {
    let merged = is_merged(root);

    if json_mode != JsonMode::Off {
        print_status_json(root, extensions, merged, json_mode);
        return;
    }

    println!("HIERARCHY   EXTENSIONS   STATUS");

    for hierarchy in DEFAULT_HIERARCHIES {
        let hier_path = root.join(hierarchy.trim_start_matches('/'));
        let exists = hier_path.is_dir();
        let is_overlay = is_overlayfs_mount(&hier_path);

        let ext_count = extensions
            .iter()
            .filter(|e| e.compatible && e.hierarchies.contains(&hierarchy.to_string()))
            .count();

        let status = if !exists {
            "not-found"
        } else if is_overlay {
            "merged"
        } else if ext_count > 0 {
            "unmerged"
        } else {
            "none"
        };

        println!("{:<12}{:<13}{}", hierarchy, ext_count, status);
    }

    if merged {
        println!();
        println!("Extensions are currently merged.");
        let merged_exts = read_merged_extensions(root);
        if !merged_exts.is_empty() {
            println!("Merged extensions:");
            for (name, path) in &merged_exts {
                println!("  {} ({})", name, path);
            }
        }
    } else {
        println!();
        println!("Extensions are not currently merged.");
    }

    if !extensions.is_empty() {
        println!();
        println!("Available extensions:");
        for ext in extensions {
            let compat = if ext.compatible { "✓" } else { "✗" };
            println!(
                "  {} {} ({}, {})",
                compat,
                ext.name,
                ext.ext_type,
                ext.path.display()
            );
        }
    }
}

fn print_status_json(root: &Path, extensions: &[Extension], merged: bool, mode: JsonMode) {
    let mut out = String::new();
    let (indent, newline) = if mode == JsonMode::Pretty {
        ("  ", "\n")
    } else {
        ("", "")
    };

    out.push('{');
    out.push_str(newline);
    out.push_str(&format!("{}\"merged\": {},{}", indent, merged, newline));

    out.push_str(&format!("{}\"hierarchies\": [", indent));
    out.push_str(newline);
    for (i, hierarchy) in DEFAULT_HIERARCHIES.iter().enumerate() {
        let hier_path = root.join(hierarchy.trim_start_matches('/'));
        let exists = hier_path.is_dir();
        let is_overlay = is_overlayfs_mount(&hier_path);

        let status = if !exists {
            "not-found"
        } else if is_overlay {
            "merged"
        } else {
            "none"
        };

        out.push_str(&format!(
            "{}{}{{\"{}\": \"{}\"}}",
            indent, indent, hierarchy, status
        ));
        if i + 1 < DEFAULT_HIERARCHIES.len() {
            out.push(',');
        }
        out.push_str(newline);
    }
    out.push_str(&format!("{}],{}", indent, newline));

    out.push_str(&format!("{}\"extensions\": [", indent));
    out.push_str(newline);
    for (i, ext) in extensions.iter().enumerate() {
        out.push_str(&format!(
            "{}{}{{\"name\": \"{}\", \"type\": \"{}\", \"path\": \"{}\", \"compatible\": {}}}",
            indent,
            indent,
            ext.name,
            ext.ext_type,
            ext.path.display(),
            ext.compatible
        ));
        if i + 1 < extensions.len() {
            out.push(',');
        }
        out.push_str(newline);
    }
    out.push_str(&format!("{}]{}", indent, newline));

    out.push('}');
    out.push_str(newline);

    print!("{}", out);
}

// ---------------------------------------------------------------------------
// Output: list
// ---------------------------------------------------------------------------

fn cmd_list(extensions: &[Extension], json_mode: JsonMode) {
    if json_mode != JsonMode::Off {
        print_list_json(extensions, json_mode);
        return;
    }

    if extensions.is_empty() {
        println!("No extensions found.");
        return;
    }

    println!(
        "{:<20} {:<10} {:<10} {:<12} PATH",
        "NAME", "TYPE", "COMPAT", "HIERARCHIES"
    );

    for ext in extensions {
        let compat = if ext.compatible { "yes" } else { "no" };
        let hier = if ext.hierarchies.is_empty() {
            "-".to_string()
        } else {
            ext.hierarchies.join(",")
        };

        println!(
            "{:<20} {:<10} {:<10} {:<12} {}",
            ext.name,
            ext.ext_type,
            compat,
            hier,
            ext.path.display()
        );
    }

    let total = extensions.len();
    let compatible = extensions.iter().filter(|e| e.compatible).count();
    println!();
    println!("{} extension(s) listed, {} compatible.", total, compatible);
}

fn print_list_json(extensions: &[Extension], mode: JsonMode) {
    let (indent, newline) = if mode == JsonMode::Pretty {
        ("  ", "\n")
    } else {
        ("", "")
    };

    let mut out = String::new();
    out.push('[');
    out.push_str(newline);

    for (i, ext) in extensions.iter().enumerate() {
        let hier_json: Vec<String> = ext
            .hierarchies
            .iter()
            .map(|h| format!("\"{}\"", h))
            .collect();

        out.push_str(&format!(
            "{}{{\"name\": \"{}\", \"type\": \"{}\", \"path\": \"{}\", \"compatible\": {}, \"hierarchies\": [{}]}}",
            indent,
            ext.name,
            ext.ext_type,
            ext.path.display(),
            ext.compatible,
            hier_json.join(", ")
        ));
        if i + 1 < extensions.len() {
            out.push(',');
        }
        out.push_str(newline);
    }

    out.push(']');
    out.push_str(newline);

    print!("{}", out);
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

fn print_usage() {
    eprintln!(
        "\
Usage: systemd-sysext [OPTIONS...] COMMAND

Activate/deactivate system extension images.

Commands:
  status           Show current extension status (default)
  list             List discovered extension images
  merge            Activate extensions (overlayfs mount)
  unmerge          Deactivate extensions (unmount)
  refresh          Unmerge and re-merge extensions
  check-inhibit    Check if merge is currently inhibited

Options:
     --root=PATH   Operate on an alternate root filesystem
     --force        Force merge even if already merged
     --no-reload    Don't reload service manager after merge/unmerge
     --no-pager     Don't pipe output through a pager
     --json=MODE    JSON output (short, pretty, off)
  -h --help         Show this help
     --version      Show version

Extension images are searched in:
  /etc/extensions/
  /run/extensions/
  /var/lib/extensions/
  /usr/lib/extensions/
  /usr/local/lib/extensions/

Each extension must contain an extension-release.<NAME> file in
usr/lib/extension-release.d/ declaring compatibility with the host OS."
    );
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn run(argv: &[String]) -> Result<(), String> {
    let args = parse_args(argv)?;

    if args.command == Command::Help {
        print_usage();
        return Ok(());
    }

    let root = &args.root;

    // Discover extensions and check compatibility
    let mut extensions = discover_extensions(root);
    let host_info = read_host_info(root);
    check_compatibility(&mut extensions, &host_info);

    match args.command {
        Command::Status => {
            cmd_status(root, &extensions, args.json_mode);
        }
        Command::List => {
            cmd_list(&extensions, args.json_mode);
        }
        Command::Merge => {
            do_merge(root, &extensions, args.force)?;
        }
        Command::Unmerge => {
            do_unmerge(root)?;
        }
        Command::Refresh => {
            if is_merged(root) {
                do_unmerge(root)?;
            }
            do_merge(root, &extensions, true)?;
        }
        Command::CheckInhibit => {
            if check_inhibit(root) {
                println!("Merge is currently inhibited.");
                process::exit(1);
            } else {
                println!("Merge is not inhibited.");
            }
        }
        Command::Help => unreachable!(),
    }

    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("systemd-sysext: {}", e);
        process::exit(1);
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_dir() -> TempDir {
        TempDir::new().expect("failed to create temp dir")
    }

    /// Create a minimal extension directory with an extension-release file.
    fn create_extension(root: &Path, ext_dir: &str, name: &str, release_vars: &[(&str, &str)]) {
        let ext_path = root.join(ext_dir.trim_start_matches('/')).join(name);
        let release_dir = ext_path.join("usr/lib/extension-release.d");
        fs::create_dir_all(&release_dir).unwrap();

        let mut content = String::new();
        for (key, value) in release_vars {
            content.push_str(&format!("{}={}\n", key, value));
        }
        fs::write(
            release_dir.join(format!("extension-release.{}", name)),
            &content,
        )
        .unwrap();
    }

    /// Create an extension with actual hierarchy content.
    fn create_extension_with_content(
        root: &Path,
        ext_dir: &str,
        name: &str,
        release_vars: &[(&str, &str)],
        hierarchies: &[(&str, &[(&str, &str)])],
    ) {
        create_extension(root, ext_dir, name, release_vars);
        let ext_path = root.join(ext_dir.trim_start_matches('/')).join(name);

        for (hier, files) in hierarchies {
            let hier_path = ext_path.join(hier.trim_start_matches('/'));
            fs::create_dir_all(&hier_path).unwrap();
            for (file, content) in *files {
                let file_path = hier_path.join(file);
                if let Some(parent) = file_path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(&file_path, content).unwrap();
            }
        }
    }

    fn setup_host(root: &Path, id: &str, version_id: &str) {
        let etc = root.join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(
            etc.join("os-release"),
            format!("ID={}\nVERSION_ID={}\n", id, version_id),
        )
        .unwrap();
    }

    // -----------------------------------------------------------------------
    // parse_args tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_args_default() {
        let args = parse_args(&[]).unwrap();
        assert_eq!(args.command, Command::Status);
        assert_eq!(args.root, PathBuf::from("/"));
        assert!(!args.force);
        assert!(!args.no_reload);
    }

    #[test]
    fn test_parse_args_status() {
        let args = parse_args(&["status".to_string()]).unwrap();
        assert_eq!(args.command, Command::Status);
    }

    #[test]
    fn test_parse_args_list() {
        let args = parse_args(&["list".to_string()]).unwrap();
        assert_eq!(args.command, Command::List);
    }

    #[test]
    fn test_parse_args_merge() {
        let args = parse_args(&["merge".to_string()]).unwrap();
        assert_eq!(args.command, Command::Merge);
    }

    #[test]
    fn test_parse_args_unmerge() {
        let args = parse_args(&["unmerge".to_string()]).unwrap();
        assert_eq!(args.command, Command::Unmerge);
    }

    #[test]
    fn test_parse_args_refresh() {
        let args = parse_args(&["refresh".to_string()]).unwrap();
        assert_eq!(args.command, Command::Refresh);
    }

    #[test]
    fn test_parse_args_check_inhibit() {
        let args = parse_args(&["check-inhibit".to_string()]).unwrap();
        assert_eq!(args.command, Command::CheckInhibit);
    }

    #[test]
    fn test_parse_args_help() {
        let args = parse_args(&["--help".to_string()]).unwrap();
        assert_eq!(args.command, Command::Help);
    }

    #[test]
    fn test_parse_args_root() {
        let args = parse_args(&["--root=/sysroot".to_string(), "list".to_string()]).unwrap();
        assert_eq!(args.root, PathBuf::from("/sysroot"));
        assert_eq!(args.command, Command::List);
    }

    #[test]
    fn test_parse_args_root_separate() {
        let args = parse_args(&[
            "--root".to_string(),
            "/sysroot".to_string(),
            "merge".to_string(),
        ])
        .unwrap();
        assert_eq!(args.root, PathBuf::from("/sysroot"));
        assert_eq!(args.command, Command::Merge);
    }

    #[test]
    fn test_parse_args_force() {
        let args = parse_args(&["--force".to_string(), "merge".to_string()]).unwrap();
        assert!(args.force);
    }

    #[test]
    fn test_parse_args_no_reload() {
        let args = parse_args(&["--no-reload".to_string(), "merge".to_string()]).unwrap();
        assert!(args.no_reload);
    }

    #[test]
    fn test_parse_args_json_short() {
        let args = parse_args(&["--json=short".to_string(), "list".to_string()]).unwrap();
        assert_eq!(args.json_mode, JsonMode::Short);
    }

    #[test]
    fn test_parse_args_json_pretty() {
        let args = parse_args(&["--json=pretty".to_string(), "status".to_string()]).unwrap();
        assert_eq!(args.json_mode, JsonMode::Pretty);
    }

    #[test]
    fn test_parse_args_json_off() {
        let args = parse_args(&["--json=off".to_string()]).unwrap();
        assert_eq!(args.json_mode, JsonMode::Off);
    }

    #[test]
    fn test_parse_args_unknown_command() {
        let result = parse_args(&["frobnicate".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_unknown_option() {
        let result = parse_args(&["--nonexistent".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_json_unknown_mode() {
        let result = parse_args(&["--json=invalid".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_no_pager_accepted() {
        let args = parse_args(&["--no-pager".to_string(), "status".to_string()]).unwrap();
        assert_eq!(args.command, Command::Status);
    }

    #[test]
    fn test_parse_args_multiple_flags() {
        let args = parse_args(&[
            "--force".to_string(),
            "--no-reload".to_string(),
            "--root=/mnt".to_string(),
            "merge".to_string(),
        ])
        .unwrap();
        assert!(args.force);
        assert!(args.no_reload);
        assert_eq!(args.root, PathBuf::from("/mnt"));
        assert_eq!(args.command, Command::Merge);
    }

    // -----------------------------------------------------------------------
    // parse_env_file tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_env_file_basic() {
        let content = "ID=nixos\nVERSION_ID=24.11\n";
        let vars = parse_env_file(content);
        assert_eq!(vars.get("ID"), Some(&"nixos".to_string()));
        assert_eq!(vars.get("VERSION_ID"), Some(&"24.11".to_string()));
    }

    #[test]
    fn test_parse_env_file_quoted() {
        let content = "ID=\"fedora\"\nNAME='Fedora Linux'\n";
        let vars = parse_env_file(content);
        assert_eq!(vars.get("ID"), Some(&"fedora".to_string()));
        assert_eq!(vars.get("NAME"), Some(&"Fedora Linux".to_string()));
    }

    #[test]
    fn test_parse_env_file_comments_blanks() {
        let content = "# Comment\n\nID=test\n   # Another\n";
        let vars = parse_env_file(content);
        assert_eq!(vars.len(), 1);
        assert_eq!(vars.get("ID"), Some(&"test".to_string()));
    }

    #[test]
    fn test_parse_env_file_empty() {
        let vars = parse_env_file("");
        assert!(vars.is_empty());
    }

    #[test]
    fn test_parse_env_file_whitespace_around_equals() {
        let content = "  KEY = value  \n";
        let vars = parse_env_file(content);
        assert_eq!(vars.get("KEY"), Some(&"value".to_string()));
    }

    #[test]
    fn test_parse_env_file_no_equals() {
        let content = "NOVALUE\nKEY=val\n";
        let vars = parse_env_file(content);
        assert_eq!(vars.len(), 1);
        assert_eq!(vars.get("KEY"), Some(&"val".to_string()));
    }

    // -----------------------------------------------------------------------
    // normalize_architecture tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_normalize_architecture() {
        assert_eq!(normalize_architecture("x86_64"), "x86-64");
        assert_eq!(normalize_architecture("amd64"), "x86-64");
        assert_eq!(normalize_architecture("aarch64"), "arm64");
        assert_eq!(normalize_architecture("arm64"), "arm64");
        assert_eq!(normalize_architecture("i686"), "x86");
        assert_eq!(normalize_architecture("armv7l"), "arm");
        assert_eq!(normalize_architecture("riscv64"), "riscv64");
        assert_eq!(normalize_architecture("s390x"), "s390x");
        assert_eq!(normalize_architecture("ppc64le"), "ppc64-le");
        assert_eq!(normalize_architecture("unknown"), "unknown");
    }

    // -----------------------------------------------------------------------
    // Extension discovery tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_discover_extensions_empty() {
        let tmp = temp_dir();
        let extensions = discover_extensions(tmp.path());
        assert!(extensions.is_empty());
    }

    #[test]
    fn test_discover_extensions_single() {
        let tmp = temp_dir();
        create_extension(
            tmp.path(),
            "/etc/extensions",
            "myext",
            &[("ID", "nixos"), ("VERSION_ID", "24.11")],
        );

        let extensions = discover_extensions(tmp.path());
        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0].name, "myext");
        assert_eq!(extensions[0].ext_type, ExtensionType::Directory);
        assert_eq!(extensions[0].release.get("ID"), Some(&"nixos".to_string()));
    }

    #[test]
    fn test_discover_extensions_multiple_dirs() {
        let tmp = temp_dir();
        create_extension(tmp.path(), "/etc/extensions", "ext-a", &[("ID", "nixos")]);
        create_extension(tmp.path(), "/run/extensions", "ext-b", &[("ID", "nixos")]);

        let extensions = discover_extensions(tmp.path());
        assert_eq!(extensions.len(), 2);
        // Sorted by name
        assert_eq!(extensions[0].name, "ext-a");
        assert_eq!(extensions[1].name, "ext-b");
    }

    #[test]
    fn test_discover_extensions_priority() {
        let tmp = temp_dir();
        // Same name in two dirs — first (higher priority) wins
        create_extension(
            tmp.path(),
            "/etc/extensions",
            "myext",
            &[("ID", "from-etc")],
        );
        create_extension(
            tmp.path(),
            "/run/extensions",
            "myext",
            &[("ID", "from-run")],
        );

        let extensions = discover_extensions(tmp.path());
        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0].name, "myext");
        assert_eq!(
            extensions[0].release.get("ID"),
            Some(&"from-etc".to_string())
        );
    }

    #[test]
    fn test_discover_extensions_hidden_files_skipped() {
        let tmp = temp_dir();
        let hidden = tmp.path().join("etc/extensions/.hidden");
        fs::create_dir_all(&hidden).unwrap();

        let extensions = discover_extensions(tmp.path());
        assert!(extensions.is_empty());
    }

    #[test]
    fn test_discover_extensions_no_release_file() {
        let tmp = temp_dir();
        let ext_path = tmp.path().join("etc/extensions/bare-ext");
        fs::create_dir_all(&ext_path).unwrap();

        let extensions = discover_extensions(tmp.path());
        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0].name, "bare-ext");
        assert!(extensions[0].release.is_empty());
    }

    #[test]
    fn test_discover_extensions_raw_image_detected() {
        let tmp = temp_dir();
        let ext_dir = tmp.path().join("etc/extensions");
        fs::create_dir_all(&ext_dir).unwrap();
        // Create a dummy .raw file
        fs::write(ext_dir.join("myext.raw"), "not a real image").unwrap();

        let extensions = discover_extensions(tmp.path());
        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0].name, "myext");
        assert_eq!(extensions[0].ext_type, ExtensionType::RawImage);
    }

    #[test]
    fn test_discover_extensions_sorted() {
        let tmp = temp_dir();
        create_extension(tmp.path(), "/etc/extensions", "zebra", &[("ID", "test")]);
        create_extension(tmp.path(), "/etc/extensions", "alpha", &[("ID", "test")]);
        create_extension(tmp.path(), "/etc/extensions", "middle", &[("ID", "test")]);

        let extensions = discover_extensions(tmp.path());
        assert_eq!(extensions.len(), 3);
        assert_eq!(extensions[0].name, "alpha");
        assert_eq!(extensions[1].name, "middle");
        assert_eq!(extensions[2].name, "zebra");
    }

    // -----------------------------------------------------------------------
    // read_extension_release tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_read_extension_release_basic() {
        let tmp = temp_dir();
        let ext_path = tmp.path().join("myext");
        let release_dir = ext_path.join("usr/lib/extension-release.d");
        fs::create_dir_all(&release_dir).unwrap();
        fs::write(
            release_dir.join("extension-release.myext"),
            "ID=nixos\nVERSION_ID=24.11\nSYSEXT_LEVEL=1.0\n",
        )
        .unwrap();

        let release = read_extension_release(&ext_path, "myext", ExtensionType::Directory);
        assert_eq!(release.get("ID"), Some(&"nixos".to_string()));
        assert_eq!(release.get("VERSION_ID"), Some(&"24.11".to_string()));
        assert_eq!(release.get("SYSEXT_LEVEL"), Some(&"1.0".to_string()));
    }

    #[test]
    fn test_read_extension_release_missing() {
        let tmp = temp_dir();
        let ext_path = tmp.path().join("norelease");
        fs::create_dir_all(&ext_path).unwrap();

        let release = read_extension_release(&ext_path, "norelease", ExtensionType::Directory);
        assert!(release.is_empty());
    }

    #[test]
    fn test_read_extension_release_single_file_fallback() {
        let tmp = temp_dir();
        let ext_path = tmp.path().join("myext");
        let release_dir = ext_path.join("usr/lib/extension-release.d");
        fs::create_dir_all(&release_dir).unwrap();
        // File name doesn't match the extension name, but it's the only one
        fs::write(release_dir.join("extension-release.other"), "ID=test\n").unwrap();

        let release = read_extension_release(&ext_path, "myext", ExtensionType::Directory);
        assert_eq!(release.get("ID"), Some(&"test".to_string()));
    }

    #[test]
    fn test_read_extension_release_raw_image_empty() {
        let tmp = temp_dir();
        let release = read_extension_release(tmp.path(), "test", ExtensionType::RawImage);
        assert!(release.is_empty());
    }

    // -----------------------------------------------------------------------
    // Compatibility checking tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_compatible_matching() {
        let host = HostInfo {
            id: Some("nixos".to_string()),
            version_id: Some("24.11".to_string()),
            sysext_level: None,
            architecture: Some("x86-64".to_string()),
        };

        let ext = Extension {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            ext_type: ExtensionType::Directory,
            release: [
                ("ID".to_string(), "nixos".to_string()),
                ("VERSION_ID".to_string(), "24.11".to_string()),
            ]
            .into_iter()
            .collect(),
            compatible: false,
            hierarchies: vec![],
        };

        assert!(is_compatible(&ext, &host));
    }

    #[test]
    fn test_is_compatible_wrong_id() {
        let host = HostInfo {
            id: Some("nixos".to_string()),
            version_id: Some("24.11".to_string()),
            sysext_level: None,
            architecture: None,
        };

        let ext = Extension {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            ext_type: ExtensionType::Directory,
            release: [
                ("ID".to_string(), "fedora".to_string()),
                ("VERSION_ID".to_string(), "24.11".to_string()),
            ]
            .into_iter()
            .collect(),
            compatible: false,
            hierarchies: vec![],
        };

        assert!(!is_compatible(&ext, &host));
    }

    #[test]
    fn test_is_compatible_wrong_version() {
        let host = HostInfo {
            id: Some("nixos".to_string()),
            version_id: Some("24.11".to_string()),
            sysext_level: None,
            architecture: None,
        };

        let ext = Extension {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            ext_type: ExtensionType::Directory,
            release: [
                ("ID".to_string(), "nixos".to_string()),
                ("VERSION_ID".to_string(), "23.05".to_string()),
            ]
            .into_iter()
            .collect(),
            compatible: false,
            hierarchies: vec![],
        };

        assert!(!is_compatible(&ext, &host));
    }

    #[test]
    fn test_is_compatible_any_id() {
        let host = HostInfo {
            id: Some("nixos".to_string()),
            version_id: Some("24.11".to_string()),
            sysext_level: None,
            architecture: None,
        };

        let ext = Extension {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            ext_type: ExtensionType::Directory,
            release: [("ID".to_string(), "_any".to_string())]
                .into_iter()
                .collect(),
            compatible: false,
            hierarchies: vec![],
        };

        assert!(is_compatible(&ext, &host));
    }

    #[test]
    fn test_is_compatible_any_architecture() {
        let host = HostInfo {
            id: Some("nixos".to_string()),
            version_id: Some("24.11".to_string()),
            sysext_level: None,
            architecture: Some("x86-64".to_string()),
        };

        let ext = Extension {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            ext_type: ExtensionType::Directory,
            release: [
                ("ID".to_string(), "nixos".to_string()),
                ("VERSION_ID".to_string(), "24.11".to_string()),
                ("ARCHITECTURE".to_string(), "_any".to_string()),
            ]
            .into_iter()
            .collect(),
            compatible: false,
            hierarchies: vec![],
        };

        assert!(is_compatible(&ext, &host));
    }

    #[test]
    fn test_is_compatible_wrong_architecture() {
        let host = HostInfo {
            id: Some("nixos".to_string()),
            version_id: Some("24.11".to_string()),
            sysext_level: None,
            architecture: Some("x86-64".to_string()),
        };

        let ext = Extension {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            ext_type: ExtensionType::Directory,
            release: [
                ("ID".to_string(), "nixos".to_string()),
                ("VERSION_ID".to_string(), "24.11".to_string()),
                ("ARCHITECTURE".to_string(), "arm64".to_string()),
            ]
            .into_iter()
            .collect(),
            compatible: false,
            hierarchies: vec![],
        };

        assert!(!is_compatible(&ext, &host));
    }

    #[test]
    fn test_is_compatible_sysext_level_overrides_version() {
        let host = HostInfo {
            id: Some("nixos".to_string()),
            version_id: Some("24.11".to_string()),
            sysext_level: Some("2.0".to_string()),
            architecture: None,
        };

        // Version doesn't match, but SYSEXT_LEVEL is present so VERSION_ID is skipped
        let ext = Extension {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            ext_type: ExtensionType::Directory,
            release: [
                ("ID".to_string(), "nixos".to_string()),
                ("VERSION_ID".to_string(), "99.99".to_string()),
                ("SYSEXT_LEVEL".to_string(), "2.0".to_string()),
            ]
            .into_iter()
            .collect(),
            compatible: false,
            hierarchies: vec![],
        };

        assert!(is_compatible(&ext, &host));
    }

    #[test]
    fn test_is_compatible_sysext_level_mismatch() {
        let host = HostInfo {
            id: Some("nixos".to_string()),
            version_id: Some("24.11".to_string()),
            sysext_level: Some("2.0".to_string()),
            architecture: None,
        };

        let ext = Extension {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            ext_type: ExtensionType::Directory,
            release: [
                ("ID".to_string(), "nixos".to_string()),
                ("SYSEXT_LEVEL".to_string(), "1.0".to_string()),
            ]
            .into_iter()
            .collect(),
            compatible: false,
            hierarchies: vec![],
        };

        assert!(!is_compatible(&ext, &host));
    }

    #[test]
    fn test_is_compatible_scope_system() {
        let host = HostInfo {
            id: Some("nixos".to_string()),
            version_id: None,
            sysext_level: None,
            architecture: None,
        };

        let ext = Extension {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            ext_type: ExtensionType::Directory,
            release: [
                ("ID".to_string(), "nixos".to_string()),
                ("SYSEXT_SCOPE".to_string(), "system portable".to_string()),
            ]
            .into_iter()
            .collect(),
            compatible: false,
            hierarchies: vec![],
        };

        assert!(is_compatible(&ext, &host));
    }

    #[test]
    fn test_is_compatible_scope_portable_only() {
        let host = HostInfo {
            id: Some("nixos".to_string()),
            version_id: None,
            sysext_level: None,
            architecture: None,
        };

        let ext = Extension {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            ext_type: ExtensionType::Directory,
            release: [
                ("ID".to_string(), "nixos".to_string()),
                ("SYSEXT_SCOPE".to_string(), "portable".to_string()),
            ]
            .into_iter()
            .collect(),
            compatible: false,
            hierarchies: vec![],
        };

        assert!(!is_compatible(&ext, &host));
    }

    #[test]
    fn test_is_compatible_no_release() {
        let host = HostInfo {
            id: Some("nixos".to_string()),
            version_id: None,
            sysext_level: None,
            architecture: None,
        };

        let ext = Extension {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            ext_type: ExtensionType::Directory,
            release: HashMap::new(),
            compatible: false,
            hierarchies: vec![],
        };

        assert!(!is_compatible(&ext, &host));
    }

    #[test]
    fn test_check_compatibility_updates_extensions() {
        let host = HostInfo {
            id: Some("nixos".to_string()),
            version_id: Some("24.11".to_string()),
            sysext_level: None,
            architecture: None,
        };

        let mut extensions = vec![
            Extension {
                name: "good".to_string(),
                path: PathBuf::from("/good"),
                ext_type: ExtensionType::Directory,
                release: [
                    ("ID".to_string(), "nixos".to_string()),
                    ("VERSION_ID".to_string(), "24.11".to_string()),
                ]
                .into_iter()
                .collect(),
                compatible: false,
                hierarchies: vec![],
            },
            Extension {
                name: "bad".to_string(),
                path: PathBuf::from("/bad"),
                ext_type: ExtensionType::Directory,
                release: [("ID".to_string(), "fedora".to_string())]
                    .into_iter()
                    .collect(),
                compatible: false,
                hierarchies: vec![],
            },
        ];

        check_compatibility(&mut extensions, &host);
        assert!(extensions[0].compatible);
        assert!(!extensions[1].compatible);
    }

    // -----------------------------------------------------------------------
    // detect_hierarchies tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_detect_hierarchies_usr() {
        let tmp = temp_dir();
        let ext_path = tmp.path().join("myext");
        fs::create_dir_all(ext_path.join("usr/bin")).unwrap();
        fs::write(ext_path.join("usr/bin/hello"), "#!/bin/sh\necho hi").unwrap();

        let hier = detect_hierarchies(&ext_path, ExtensionType::Directory);
        assert!(hier.contains(&"/usr".to_string()));
    }

    #[test]
    fn test_detect_hierarchies_opt() {
        let tmp = temp_dir();
        let ext_path = tmp.path().join("myext");
        fs::create_dir_all(ext_path.join("opt/myapp")).unwrap();
        fs::write(ext_path.join("opt/myapp/run"), "#!/bin/sh").unwrap();

        let hier = detect_hierarchies(&ext_path, ExtensionType::Directory);
        assert!(hier.contains(&"/opt".to_string()));
    }

    #[test]
    fn test_detect_hierarchies_both() {
        let tmp = temp_dir();
        let ext_path = tmp.path().join("myext");
        fs::create_dir_all(ext_path.join("usr/bin")).unwrap();
        fs::create_dir_all(ext_path.join("opt/app")).unwrap();
        fs::write(ext_path.join("usr/bin/hello"), "").unwrap();
        fs::write(ext_path.join("opt/app/run"), "").unwrap();

        let hier = detect_hierarchies(&ext_path, ExtensionType::Directory);
        assert_eq!(hier.len(), 2);
        assert!(hier.contains(&"/usr".to_string()));
        assert!(hier.contains(&"/opt".to_string()));
    }

    #[test]
    fn test_detect_hierarchies_empty() {
        let tmp = temp_dir();
        let ext_path = tmp.path().join("myext");
        fs::create_dir_all(&ext_path).unwrap();

        let hier = detect_hierarchies(&ext_path, ExtensionType::Directory);
        assert!(hier.is_empty());
    }

    #[test]
    fn test_detect_hierarchies_raw_image() {
        let tmp = temp_dir();
        let hier = detect_hierarchies(tmp.path(), ExtensionType::RawImage);
        assert!(hier.is_empty());
    }

    // -----------------------------------------------------------------------
    // Merge marker tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_merge_marker_roundtrip() {
        let tmp = temp_dir();

        assert!(!is_merged(tmp.path()));

        let extensions = vec![
            Extension {
                name: "ext1".to_string(),
                path: PathBuf::from("/etc/extensions/ext1"),
                ext_type: ExtensionType::Directory,
                release: HashMap::new(),
                compatible: true,
                hierarchies: vec![],
            },
            Extension {
                name: "ext2".to_string(),
                path: PathBuf::from("/run/extensions/ext2"),
                ext_type: ExtensionType::Directory,
                release: HashMap::new(),
                compatible: true,
                hierarchies: vec![],
            },
        ];

        write_merge_marker(tmp.path(), &extensions).unwrap();
        assert!(is_merged(tmp.path()));

        let merged = read_merged_extensions(tmp.path());
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].0, "ext1");
        assert_eq!(merged[0].1, "/etc/extensions/ext1");
        assert_eq!(merged[1].0, "ext2");
        assert_eq!(merged[1].1, "/run/extensions/ext2");

        remove_merge_marker(tmp.path()).unwrap();
        assert!(!is_merged(tmp.path()));
    }

    #[test]
    fn test_merge_marker_empty_extensions() {
        let tmp = temp_dir();
        write_merge_marker(tmp.path(), &[]).unwrap();
        assert!(is_merged(tmp.path()));

        let merged = read_merged_extensions(tmp.path());
        assert!(merged.is_empty());
    }

    #[test]
    fn test_remove_merge_marker_nonexistent() {
        let tmp = temp_dir();
        // Should not error if marker doesn't exist
        remove_merge_marker(tmp.path()).unwrap();
    }

    // -----------------------------------------------------------------------
    // Check inhibit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_check_inhibit_not_inhibited() {
        let tmp = temp_dir();
        assert!(!check_inhibit(tmp.path()));
    }

    #[test]
    fn test_check_inhibit_inhibited() {
        let tmp = temp_dir();
        let inhibit_path = tmp.path().join("run/systemd/sysext/inhibit");
        fs::create_dir_all(inhibit_path.parent().unwrap()).unwrap();
        fs::write(&inhibit_path, "inhibited\n").unwrap();
        assert!(check_inhibit(tmp.path()));
    }

    // -----------------------------------------------------------------------
    // read_host_info tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_read_host_info_from_etc() {
        let tmp = temp_dir();
        setup_host(tmp.path(), "nixos", "24.11");

        let info = read_host_info(tmp.path());
        assert_eq!(info.id, Some("nixos".to_string()));
        assert_eq!(info.version_id, Some("24.11".to_string()));
    }

    #[test]
    fn test_read_host_info_from_usr_lib() {
        let tmp = temp_dir();
        let usr_lib = tmp.path().join("usr/lib");
        fs::create_dir_all(&usr_lib).unwrap();
        fs::write(usr_lib.join("os-release"), "ID=fedora\nVERSION_ID=41\n").unwrap();

        let info = read_host_info(tmp.path());
        assert_eq!(info.id, Some("fedora".to_string()));
        assert_eq!(info.version_id, Some("41".to_string()));
    }

    #[test]
    fn test_read_host_info_etc_takes_priority() {
        let tmp = temp_dir();
        setup_host(tmp.path(), "nixos", "24.11");

        let usr_lib = tmp.path().join("usr/lib");
        fs::create_dir_all(&usr_lib).unwrap();
        fs::write(usr_lib.join("os-release"), "ID=fedora\nVERSION_ID=41\n").unwrap();

        let info = read_host_info(tmp.path());
        assert_eq!(info.id, Some("nixos".to_string()));
        assert_eq!(info.version_id, Some("24.11".to_string()));
    }

    #[test]
    fn test_read_host_info_missing() {
        let tmp = temp_dir();
        let info = read_host_info(tmp.path());
        assert!(info.id.is_none());
        assert!(info.version_id.is_none());
    }

    #[test]
    fn test_read_host_info_with_sysext_level() {
        let tmp = temp_dir();
        let etc = tmp.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(
            etc.join("os-release"),
            "ID=nixos\nVERSION_ID=24.11\nSYSEXT_LEVEL=2.0\n",
        )
        .unwrap();

        let info = read_host_info(tmp.path());
        assert_eq!(info.sysext_level, Some("2.0".to_string()));
    }

    // -----------------------------------------------------------------------
    // Integration-level tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_discover_and_check_compatible() {
        let tmp = temp_dir();
        setup_host(tmp.path(), "nixos", "24.11");

        create_extension(
            tmp.path(),
            "/etc/extensions",
            "good-ext",
            &[("ID", "nixos"), ("VERSION_ID", "24.11")],
        );
        create_extension(
            tmp.path(),
            "/etc/extensions",
            "bad-ext",
            &[("ID", "fedora"), ("VERSION_ID", "41")],
        );
        create_extension(tmp.path(), "/etc/extensions", "any-ext", &[("ID", "_any")]);

        let mut extensions = discover_extensions(tmp.path());
        let host_info = read_host_info(tmp.path());
        check_compatibility(&mut extensions, &host_info);

        // Sorted: any-ext, bad-ext, good-ext
        assert_eq!(extensions.len(), 3);
        assert_eq!(extensions[0].name, "any-ext");
        assert!(extensions[0].compatible);
        assert_eq!(extensions[1].name, "bad-ext");
        assert!(!extensions[1].compatible);
        assert_eq!(extensions[2].name, "good-ext");
        assert!(extensions[2].compatible);
    }

    #[test]
    fn test_discover_with_content_hierarchies() {
        let tmp = temp_dir();
        setup_host(tmp.path(), "nixos", "24.11");

        create_extension_with_content(
            tmp.path(),
            "/etc/extensions",
            "myext",
            &[("ID", "nixos"), ("VERSION_ID", "24.11")],
            &[("usr", &[("bin/hello", "#!/bin/sh\necho hello")])],
        );

        let mut extensions = discover_extensions(tmp.path());
        let host_info = read_host_info(tmp.path());
        check_compatibility(&mut extensions, &host_info);

        assert_eq!(extensions.len(), 1);
        assert!(extensions[0].compatible);
        assert!(extensions[0].hierarchies.contains(&"/usr".to_string()));
    }

    #[test]
    fn test_do_merge_no_compatible() {
        let tmp = temp_dir();
        setup_host(tmp.path(), "nixos", "24.11");

        // Create extension that won't be compatible
        create_extension(
            tmp.path(),
            "/etc/extensions",
            "bad-ext",
            &[("ID", "fedora")],
        );

        let mut extensions = discover_extensions(tmp.path());
        let host_info = read_host_info(tmp.path());
        check_compatibility(&mut extensions, &host_info);

        let result = do_merge(tmp.path(), &extensions, false);
        assert!(result.is_ok());
        // No marker should be written when there are no compatible extensions
        assert!(!is_merged(tmp.path()));
    }

    #[test]
    fn test_do_merge_already_merged() {
        let tmp = temp_dir();
        write_merge_marker(tmp.path(), &[]).unwrap();

        let result = do_merge(tmp.path(), &[], false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already merged"));
    }

    #[test]
    fn test_do_unmerge_not_merged() {
        let tmp = temp_dir();
        // Should succeed silently
        let result = do_unmerge(tmp.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_do_unmerge_removes_marker() {
        let tmp = temp_dir();
        write_merge_marker(tmp.path(), &[]).unwrap();
        assert!(is_merged(tmp.path()));

        let result = do_unmerge(tmp.path());
        assert!(result.is_ok());
        assert!(!is_merged(tmp.path()));
    }

    // -----------------------------------------------------------------------
    // ExtensionType Display tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_extension_type_display() {
        assert_eq!(format!("{}", ExtensionType::Directory), "directory");
        assert_eq!(format!("{}", ExtensionType::RawImage), "raw");
    }

    // -----------------------------------------------------------------------
    // run() tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_run_status_empty() {
        let tmp = temp_dir();
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "status".to_string(),
        ];
        let result = run(&args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_list_empty() {
        let tmp = temp_dir();
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "list".to_string(),
        ];
        let result = run(&args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_check_inhibit_not_inhibited() {
        let tmp = temp_dir();
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "check-inhibit".to_string(),
        ];
        // check-inhibit exits with code 0 if not inhibited, but our run()
        // just returns Ok since we handle exit codes in main
        let result = run(&args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_list_with_extensions() {
        let tmp = temp_dir();
        setup_host(tmp.path(), "nixos", "24.11");
        create_extension(
            tmp.path(),
            "/etc/extensions",
            "myext",
            &[("ID", "nixos"), ("VERSION_ID", "24.11")],
        );

        let args = vec![
            format!("--root={}", tmp.path().display()),
            "list".to_string(),
        ];
        let result = run(&args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_list_json() {
        let tmp = temp_dir();
        setup_host(tmp.path(), "nixos", "24.11");
        create_extension(
            tmp.path(),
            "/etc/extensions",
            "myext",
            &[("ID", "nixos"), ("VERSION_ID", "24.11")],
        );

        let args = vec![
            format!("--root={}", tmp.path().display()),
            "--json=short".to_string(),
            "list".to_string(),
        ];
        let result = run(&args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_status_json_pretty() {
        let tmp = temp_dir();
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "--json=pretty".to_string(),
            "status".to_string(),
        ];
        let result = run(&args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_unmerge_when_not_merged() {
        let tmp = temp_dir();
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "unmerge".to_string(),
        ];
        let result = run(&args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_merge_no_extensions() {
        let tmp = temp_dir();
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "merge".to_string(),
        ];
        let result = run(&args);
        assert!(result.is_ok());
    }
}

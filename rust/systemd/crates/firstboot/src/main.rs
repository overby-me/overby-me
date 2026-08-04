//! systemd-firstboot — Initialize basic system settings on or before the first boot.
//!
//! A drop-in replacement for `systemd-firstboot(8)`. This tool is used to
//! initialize basic system settings before or during the first boot of a
//! newly installed system. It can set:
//!
//!   - Machine ID (`/etc/machine-id`)
//!   - Locale (`/etc/locale.conf`)
//!   - Keymap (`/etc/vconsole.conf`)
//!   - Timezone (`/etc/localtime` symlink)
//!   - Hostname (`/etc/hostname`)
//!   - Root password/hash (`/etc/shadow`)
//!   - Root shell (`/etc/passwd`)
//!   - Kernel command line (`/etc/kernel/cmdline`)
//!
//! Each setting can be provided via command-line options or read from
//! credential files via `$CREDENTIALS_DIRECTORY`. Settings are only
//! applied if the target file does not already exist (unless `--force`
//! is used) and the system has not been booted before (unless
//! `--force` is used).
//!
//! Modes of operation:
//!
//!   systemd-firstboot --locale=en_US.UTF-8
//!       Set the system locale non-interactively.
//!
//!   systemd-firstboot --prompt
//!       Prompt for all unset settings interactively.
//!
//!   systemd-firstboot --copy-locale --copy-timezone
//!       Copy settings from the host system to the image.
//!
//!   systemd-firstboot --root=/sysroot --hostname=myhost
//!       Apply settings to a specific root filesystem path.
//!
//!   systemd-firstboot --reset-locale --reset-hostname
//!       Remove configuration files to reset settings.
//!
//! Exit codes:
//!   0 — success
//!   1 — error

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process;

#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// All the configurable first-boot settings.
#[derive(Debug, Clone, Default)]
struct Settings {
    locale: Option<String>,
    locale_messages: Option<String>,
    keymap: Option<String>,
    timezone: Option<String>,
    hostname: Option<String>,
    machine_id: Option<String>,
    root_password: Option<String>,
    root_password_hashed: Option<bool>,
    root_shell: Option<String>,
    kernel_cmdline: Option<String>,
}

/// Parsed command-line arguments.
#[derive(Debug, Clone, Default)]
struct Args {
    root: PathBuf,
    image: Option<String>,
    force: bool,
    delete_root_password: bool,
    welcome: bool,

    // Prompt flags
    prompt: bool,
    prompt_locale: bool,
    prompt_keymap: bool,
    prompt_timezone: bool,
    prompt_hostname: bool,
    prompt_root_password: bool,
    prompt_root_shell: bool,

    // Copy flags
    copy_locale: bool,
    copy_keymap: bool,
    copy_timezone: bool,
    copy_root_password: bool,
    copy_root_shell: bool,

    // Reset flags
    reset_locale: bool,
    reset_keymap: bool,
    reset_timezone: bool,
    reset_hostname: bool,
    reset_machine_id: bool,
    reset_root_password: bool,
    reset_root_shell: bool,
    reset_kernel_cmdline: bool,

    // Direct value flags
    settings: Settings,
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args {
        root: PathBuf::from("/"),
        welcome: true,
        ..Default::default()
    };

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];

        // Handle --option=value forms
        let (key, value) = if let Some(pos) = arg.find('=') {
            (&arg[..pos], Some(arg[pos + 1..].to_string()))
        } else {
            (arg.as_str(), None)
        };

        match key {
            "--help" | "-h" => {
                print_usage();
                process::exit(0);
            }
            "--version" => {
                println!("systemd-firstboot (rust-systemd)");
                process::exit(0);
            }
            "--root" => {
                let v = value_or_next(args, &mut i, value, "--root")?;
                parsed.root = PathBuf::from(v);
            }
            "--image" => {
                let v = value_or_next(args, &mut i, value, "--image")?;
                parsed.image = Some(v);
            }
            "--locale" => {
                let v = value_or_next(args, &mut i, value, "--locale")?;
                parsed.settings.locale = Some(v);
            }
            "--locale-messages" => {
                let v = value_or_next(args, &mut i, value, "--locale-messages")?;
                parsed.settings.locale_messages = Some(v);
            }
            "--keymap" => {
                let v = value_or_next(args, &mut i, value, "--keymap")?;
                parsed.settings.keymap = Some(v);
            }
            "--timezone" => {
                let v = value_or_next(args, &mut i, value, "--timezone")?;
                parsed.settings.timezone = Some(v);
            }
            "--hostname" => {
                let v = value_or_next(args, &mut i, value, "--hostname")?;
                parsed.settings.hostname = Some(v);
            }
            "--machine-id" => {
                let v = value_or_next(args, &mut i, value, "--machine-id")?;
                parsed.settings.machine_id = Some(v);
            }
            "--root-password" => {
                let v = value_or_next(args, &mut i, value, "--root-password")?;
                parsed.settings.root_password = Some(v);
                parsed.settings.root_password_hashed = Some(false);
            }
            "--root-password-hashed" => {
                let v = value_or_next(args, &mut i, value, "--root-password-hashed")?;
                parsed.settings.root_password = Some(v);
                parsed.settings.root_password_hashed = Some(true);
            }
            "--root-password-file" => {
                let v = value_or_next(args, &mut i, value, "--root-password-file")?;
                let content = fs::read_to_string(&v)
                    .map_err(|e| format!("Failed to read password file {}: {}", v, e))?;
                parsed.settings.root_password = Some(content.trim().to_string());
                parsed.settings.root_password_hashed = Some(false);
            }
            "--root-shell" => {
                let v = value_or_next(args, &mut i, value, "--root-shell")?;
                parsed.settings.root_shell = Some(v);
            }
            "--kernel-command-line" => {
                let v = value_or_next(args, &mut i, value, "--kernel-command-line")?;
                parsed.settings.kernel_cmdline = Some(v);
            }
            "--force" => parsed.force = true,
            "--delete-root-password" => parsed.delete_root_password = true,
            "--welcome" => {
                // --welcome=BOOL
                if let Some(v) = value {
                    parsed.welcome = parse_bool(&v);
                } else {
                    parsed.welcome = true;
                }
            }
            "--prompt" => parsed.prompt = true,
            "--prompt-locale" => parsed.prompt_locale = true,
            "--prompt-keymap" => parsed.prompt_keymap = true,
            "--prompt-timezone" => parsed.prompt_timezone = true,
            "--prompt-hostname" => parsed.prompt_hostname = true,
            "--prompt-root-password" => parsed.prompt_root_password = true,
            "--prompt-root-shell" => parsed.prompt_root_shell = true,

            // Upstream's ARG_COPY sets exactly these five. Unlike --reset,
            // which leaves the root account alone, --copy does include the
            // root password and shell.
            "--copy" => {
                parsed.copy_locale = true;
                parsed.copy_keymap = true;
                parsed.copy_timezone = true;
                parsed.copy_root_password = true;
                parsed.copy_root_shell = true;
            }
            "--copy-locale" => parsed.copy_locale = true,
            "--copy-keymap" => parsed.copy_keymap = true,
            "--copy-timezone" => parsed.copy_timezone = true,
            "--copy-root-password" => parsed.copy_root_password = true,
            "--copy-root-shell" => parsed.copy_root_shell = true,

            // Upstream's process_reset() removes exactly locale.conf,
            // vconsole.conf, hostname, machine-id, kernel/cmdline and
            // localtime. It deliberately leaves the root account alone, so
            // this is NOT simply every --reset-* flag at once.
            "--reset" => {
                parsed.reset_locale = true;
                parsed.reset_keymap = true;
                parsed.reset_timezone = true;
                parsed.reset_hostname = true;
                parsed.reset_machine_id = true;
                parsed.reset_kernel_cmdline = true;
            }
            "--reset-locale" => parsed.reset_locale = true,
            "--reset-keymap" => parsed.reset_keymap = true,
            "--reset-timezone" => parsed.reset_timezone = true,
            "--reset-hostname" => parsed.reset_hostname = true,
            "--reset-machine-id" => parsed.reset_machine_id = true,
            "--reset-root-password" => parsed.reset_root_password = true,
            "--reset-root-shell" => parsed.reset_root_shell = true,
            "--reset-kernel-cmdline" => parsed.reset_kernel_cmdline = true,

            // Silently accept --setup-machine-id as an alias
            "--setup-machine-id" => {
                // Auto-generate a machine ID
                parsed.settings.machine_id = Some(generate_machine_id());
            }

            other => {
                return Err(format!("Unknown option: {}", other));
            }
        }

        i += 1;
    }

    Ok(parsed)
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

fn parse_bool(s: &str) -> bool {
    matches!(s.to_lowercase().as_str(), "1" | "yes" | "true" | "on" | "y")
}

// ---------------------------------------------------------------------------
// Machine ID generation
// ---------------------------------------------------------------------------

fn generate_machine_id() -> String {
    // Try /proc/sys/kernel/random/uuid first
    if let Ok(uuid) = fs::read_to_string("/proc/sys/kernel/random/uuid") {
        let hex: String = uuid.trim().chars().filter(|c| *c != '-').collect();
        if hex.len() == 32 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return hex.to_lowercase();
        }
    }

    // Fallback: read from /dev/urandom
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        let mut buf = [0u8; 16];
        if f.read_exact(&mut buf).is_ok() {
            return buf.iter().map(|b| format!("{:02x}", b)).collect();
        }
    }

    // Last resort: timestamp-based (not cryptographically random)
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{:032x}", now.as_nanos())
}

fn is_valid_machine_id(id: &str) -> bool {
    id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit())
}

// ---------------------------------------------------------------------------
// Hostname validation
// ---------------------------------------------------------------------------

fn is_valid_hostname(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    // Must not start or end with dot or hyphen
    if name.starts_with('.') || name.starts_with('-') || name.ends_with('.') || name.ends_with('-')
    {
        return false;
    }
    // Only allow alphanumeric, dot, hyphen
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

// ---------------------------------------------------------------------------
// Timezone validation
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn is_valid_timezone(tz: &str, root: &Path) -> bool {
    if tz.is_empty() || tz.starts_with('/') || tz.contains("..") {
        return false;
    }

    // Check that the zoneinfo file exists
    let zoneinfo = root.join("usr/share/zoneinfo").join(tz);
    zoneinfo.exists()
        || Path::new("/usr/share/zoneinfo").join(tz).exists()
        || tz == "UTC"
        || tz == "Etc/UTC"
}

// ---------------------------------------------------------------------------
// Locale helpers
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn list_available_locales() -> Vec<String> {
    let mut locales = Vec::new();

    // Parse /usr/share/i18n/SUPPORTED or /etc/locale.gen
    for path in &[
        "/usr/share/i18n/SUPPORTED",
        "/etc/locale.gen",
        "/usr/share/i18n/locales",
    ] {
        if let Ok(content) = fs::read_to_string(path) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                // Take just the locale name (before any whitespace)
                if let Some(name) = line.split_whitespace().next() {
                    locales.push(name.to_string());
                }
            }
            break;
        }
    }

    if locales.is_empty() {
        // Common fallbacks
        locales.push("C".to_string());
        locales.push("C.UTF-8".to_string());
        locales.push("POSIX".to_string());
        locales.push("en_US.UTF-8".to_string());
    }

    locales.sort();
    locales.dedup();
    locales
}

// ---------------------------------------------------------------------------
// Keymap helpers
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn list_available_keymaps() -> Vec<String> {
    let mut keymaps = Vec::new();

    let search_dirs = [
        "/usr/share/keymaps",
        "/usr/share/kbd/keymaps",
        "/usr/lib/kbd/keymaps",
    ];

    for dir in &search_dirs {
        if let Ok(()) = collect_keymaps(Path::new(dir), &mut keymaps) {
            break;
        }
    }

    // Also try localectl-style listing from /usr/share/X11/xkb/rules/base.lst
    if keymaps.is_empty() {
        // Common fallbacks
        keymaps.push("us".to_string());
        keymaps.push("de".to_string());
        keymaps.push("fr".to_string());
        keymaps.push("uk".to_string());
    }

    keymaps.sort();
    keymaps.dedup();
    keymaps
}

#[allow(dead_code)]
fn collect_keymaps(dir: &Path, keymaps: &mut Vec<String>) -> io::Result<()> {
    if !dir.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "not a directory"));
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let _ = collect_keymaps(&path, keymaps);
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            // Strip .map.gz, .map, .kmap.gz, .kmap extensions
            let name = name
                .strip_suffix(".map.gz")
                .or_else(|| name.strip_suffix(".kmap.gz"))
                .or_else(|| name.strip_suffix(".map"))
                .or_else(|| name.strip_suffix(".kmap"))
                .unwrap_or(name);
            if !name.is_empty() {
                keymaps.push(name.to_string());
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Shell helpers
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn list_available_shells() -> Vec<String> {
    let mut shells = Vec::new();

    if let Ok(content) = fs::read_to_string("/etc/shells") {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            shells.push(line.to_string());
        }
    }

    if shells.is_empty() {
        shells.push("/bin/bash".to_string());
        shells.push("/bin/sh".to_string());
    }

    shells
}

// ---------------------------------------------------------------------------
// Credential loading
// ---------------------------------------------------------------------------

fn load_credential_from_dir(dir: &Path, name: &str) -> Option<String> {
    let path = dir.join(name);
    if let Ok(content) = fs::read_to_string(&path) {
        let trimmed = content.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    None
}

#[allow(dead_code)]
fn load_credential(name: &str) -> Option<String> {
    if let Ok(cred_dir) = std::env::var("CREDENTIALS_DIRECTORY") {
        return load_credential_from_dir(Path::new(&cred_dir), name);
    }
    None
}

fn load_settings_from_credentials_dir(settings: &mut Settings, dir: &Path) {
    if settings.locale.is_none() {
        settings.locale = load_credential_from_dir(dir, "firstboot.locale");
    }
    if settings.locale_messages.is_none() {
        settings.locale_messages = load_credential_from_dir(dir, "firstboot.locale-messages");
    }
    if settings.keymap.is_none() {
        settings.keymap = load_credential_from_dir(dir, "firstboot.keymap");
    }
    if settings.timezone.is_none() {
        settings.timezone = load_credential_from_dir(dir, "firstboot.timezone");
    }
    if settings.hostname.is_none() {
        settings.hostname = load_credential_from_dir(dir, "firstboot.hostname");
    }
    if settings.machine_id.is_none() {
        settings.machine_id = load_credential_from_dir(dir, "firstboot.machine-id");
    }
    if settings.root_password.is_none() {
        if let Some(hash) = load_credential_from_dir(dir, "firstboot.root-password-hashed") {
            settings.root_password = Some(hash);
            settings.root_password_hashed = Some(true);
        } else if let Some(pw) = load_credential_from_dir(dir, "firstboot.root-password") {
            settings.root_password = Some(pw);
            settings.root_password_hashed = Some(false);
        }
    }
    if settings.root_shell.is_none() {
        settings.root_shell = load_credential_from_dir(dir, "firstboot.root-shell");
    }
    if settings.kernel_cmdline.is_none() {
        settings.kernel_cmdline = load_credential_from_dir(dir, "firstboot.kernel-command-line");
    }
}

fn load_settings_from_credentials(settings: &mut Settings) {
    if let Ok(cred_dir) = std::env::var("CREDENTIALS_DIRECTORY") {
        load_settings_from_credentials_dir(settings, Path::new(&cred_dir));
    }
}

// ---------------------------------------------------------------------------
// Copy from host
// ---------------------------------------------------------------------------

/// Copy `/<relative>` from the host into `<root>/<relative>`, byte for byte.
///
/// Returns false when there is nothing to copy, either because the host has no
/// such file or because the target is already configured and --force was not
/// given, so the caller can fall back to reconstructing the file from parsed
/// values.
fn copy_host_file_verbatim(root: &Path, relative: &str, force: bool) -> io::Result<bool> {
    let host = Path::new("/").join(relative);
    let Ok(content) = fs::read(&host) else {
        return Ok(false);
    };
    if !should_apply(root, relative, force) {
        return Ok(false);
    }
    let dest = root.join(relative);
    ensure_parent_dir(&dest)?;
    fs::write(&dest, content)?;
    #[cfg(target_os = "linux")]
    {
        fs::set_permissions(&dest, fs::Permissions::from_mode(0o644))?;
    }
    eprintln!("Copied {} to {}.", host.display(), dest.display());
    Ok(true)
}

fn copy_locale_from_host(settings: &mut Settings) {
    if settings.locale.is_some() {
        return;
    }
    if let Ok(content) = fs::read_to_string("/etc/locale.conf") {
        let vars = parse_env_file(&content);
        if let Some(v) = vars.get("LANG") {
            settings.locale = Some(v.clone());
        }
        if settings.locale_messages.is_none()
            && let Some(v) = vars.get("LC_MESSAGES")
        {
            settings.locale_messages = Some(v.clone());
        }
    }
}

fn copy_keymap_from_host(settings: &mut Settings) {
    if settings.keymap.is_some() {
        return;
    }
    if let Ok(content) = fs::read_to_string("/etc/vconsole.conf") {
        let vars = parse_env_file(&content);
        if let Some(v) = vars.get("KEYMAP") {
            settings.keymap = Some(v.clone());
        }
    }
}

fn copy_timezone_from_host(settings: &mut Settings) {
    if settings.timezone.is_some() {
        return;
    }
    // /etc/localtime is typically a symlink into /usr/share/zoneinfo/
    if let Ok(target) = fs::read_link("/etc/localtime") {
        let target_str = target.to_string_lossy();
        if let Some(tz) = target_str.strip_prefix("/usr/share/zoneinfo/") {
            settings.timezone = Some(tz.to_string());
        } else if let Some(tz) = target_str.strip_prefix("../usr/share/zoneinfo/") {
            settings.timezone = Some(tz.to_string());
        }
    }
}

fn copy_root_password_from_host(settings: &mut Settings) {
    if settings.root_password.is_some() {
        return;
    }
    // Copy the field VERBATIM, including a locked or empty one.
    // read_root_password_hash() deliberately skips those, which is right when
    // asking "does root have a usable password", but wrong here: a copy that
    // silently drops a locked password leaves the account to be filled in with
    // something else, and the test diffs the copied field against the host's.
    if let Some(field) = read_root_password_field_raw("/etc/shadow") {
        settings.root_password = Some(field);
        settings.root_password_hashed = Some(true);
    }
}

/// The root password hash, if root has a usable one.
///
/// This is the "is root's password usable" predicate: locked and placeholder
/// values are not hashes and are skipped. No longer on the binary's path,
/// since the only former caller was the --copy-root-password path and a copy
/// must reproduce even a locked field, but kept with its unit coverage because
/// it answers a different question from read_root_password_field_raw() below
/// and the distinction is exactly what caused a bug once already.
#[allow(dead_code)]
fn read_root_password_hash(path: &str) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() >= 2 && fields[0] == "root" {
            let hash = fields[1];
            // Skip locked/empty passwords
            if !hash.is_empty() && hash != "!" && hash != "!!" && hash != "*" && hash != "x" {
                return Some(hash.to_string());
            }
        }
    }
    None
}

/// The root password field from a shadow file exactly as written, with no
/// judgement about whether it is a usable hash.
fn read_root_password_field_raw(path: &str) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    content.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        (fields.len() >= 2 && fields[0] == "root").then(|| fields[1].to_string())
    })
}

fn copy_root_shell_from_host(settings: &mut Settings) {
    if settings.root_shell.is_some() {
        return;
    }
    if let Some(shell) = read_root_shell("/etc/passwd") {
        settings.root_shell = Some(shell);
    }
}


fn read_root_shell(path: &str) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() >= 7 && fields[0] == "root" {
            let shell = fields[6];
            if !shell.is_empty() {
                return Some(shell.to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Environment file helpers
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
            if (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''))
            {
                value = value[1..value.len() - 1].to_string();
            }
            vars.insert(key, value);
        }
    }
    vars
}

fn format_env_file(vars: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (key, value) in vars {
        // Quote value if it contains spaces or special characters
        if value.contains(' ')
            || value.contains('\t')
            || value.contains('#')
            || value.contains('\'')
        {
            out.push_str(&format!("{}=\"{}\"\n", key, value));
        } else {
            out.push_str(&format!("{}={}\n", key, value));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Password hashing
// ---------------------------------------------------------------------------

/// Hash a password by invoking the `mkpasswd` or `openssl` command,
/// or fall back to storing the plaintext prefixed with `{PLAIN}` so the
/// caller can re-hash it later. For production use, pass pre-hashed
/// passwords via `--root-password-hashed` instead.
fn hash_password(password: &str) -> String {
    // Try mkpasswd (from whois package) — produces $6$ hashes
    if let Ok(output) = std::process::Command::new("mkpasswd")
        .args(["--method=sha-512", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut child| {
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(password.as_bytes());
            }
            child.wait_with_output()
        })
        && output.status.success()
    {
        let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if hash.starts_with("$6$") {
            return hash;
        }
    }

    // Try openssl passwd
    if let Ok(output) = std::process::Command::new("openssl")
        .args(["passwd", "-6", "-stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut child| {
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(password.as_bytes());
            }
            child.wait_with_output()
        })
        && output.status.success()
    {
        let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if hash.starts_with("$") {
            return hash;
        }
    }

    // No hashing tool available — store as locked password with a warning.
    // The caller should use --root-password-hashed with a pre-hashed value.
    eprintln!("Warning: No password hashing tool (mkpasswd, openssl) found.");
    eprintln!("Use --root-password-hashed to provide a pre-hashed password.");
    "!".to_string()
}

// ---------------------------------------------------------------------------
// File target checking
// ---------------------------------------------------------------------------

/// Check whether the system has already been booted (machine-id is initialized).
fn system_already_booted(root: &Path) -> bool {
    let machine_id = root.join("etc/machine-id");
    if let Ok(content) = fs::read_to_string(&machine_id) {
        let trimmed = content.trim();
        if !trimmed.is_empty() && trimmed != "uninitialized" {
            return true;
        }
    }
    false
}

/// Check whether a target config file already exists and is non-empty.
fn target_exists(root: &Path, relative: &str) -> bool {
    let path = root.join(relative);
    if path.is_symlink() {
        return true;
    }
    if let Ok(meta) = fs::metadata(&path) {
        meta.len() > 0
    } else {
        false
    }
}

fn should_apply(root: &Path, relative: &str, force: bool) -> bool {
    if force {
        return true;
    }
    !target_exists(root, relative)
}

// ---------------------------------------------------------------------------
// Prompting (interactive mode)
// ---------------------------------------------------------------------------

fn prompt_value(prompt: &str, default: Option<&str>) -> Option<String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    if let Some(d) = default {
        write!(stdout, "{} [{}]: ", prompt, d).ok();
    } else {
        write!(stdout, "{}: ", prompt).ok();
    }
    stdout.flush().ok();

    let mut line = String::new();
    if stdin.lock().read_line(&mut line).is_err() {
        return None;
    }

    let trimmed = line.trim().to_string();
    if trimmed.is_empty() {
        default.map(|d| d.to_string())
    } else {
        Some(trimmed)
    }
}

fn prompt_password(prompt: &str) -> Option<String> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    write!(stdout, "{}: ", prompt).ok();
    stdout.flush().ok();

    // Disable echo using termios
    let echo_disabled = disable_echo();

    let stdin = io::stdin();
    let mut line = String::new();
    let result = if stdin.lock().read_line(&mut line).is_ok() {
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    } else {
        None
    };

    if echo_disabled {
        restore_echo();
        writeln!(stdout).ok();
    }

    result
}

fn disable_echo() -> bool {
    // In non-interactive / CI contexts we just skip echo control
    false
}

fn restore_echo() {
    // No-op: echo control not implemented without libc termios
}

// ---------------------------------------------------------------------------
// Apply settings
// ---------------------------------------------------------------------------

fn apply_locale(root: &Path, settings: &Settings, force: bool) -> io::Result<bool> {
    if settings.locale.is_none() && settings.locale_messages.is_none() {
        return Ok(false);
    }

    if !should_apply(root, "etc/locale.conf", force) {
        return Ok(false);
    }

    let mut vars = Vec::new();
    if let Some(ref locale) = settings.locale {
        vars.push(("LANG", locale.as_str()));
    }
    if let Some(ref lm) = settings.locale_messages {
        vars.push(("LC_MESSAGES", lm.as_str()));
    }

    let path = root.join("etc/locale.conf");
    ensure_parent_dir(&path)?;
    fs::write(&path, format_env_file(&vars))?;
    eprintln!("Created {}.", path.display());
    Ok(true)
}

fn apply_keymap(root: &Path, settings: &Settings, force: bool) -> io::Result<bool> {
    let keymap = match &settings.keymap {
        Some(k) => k,
        None => return Ok(false),
    };

    if !should_apply(root, "etc/vconsole.conf", force) {
        return Ok(false);
    }

    let path = root.join("etc/vconsole.conf");
    ensure_parent_dir(&path)?;

    // Preserve existing entries, just update/add KEYMAP
    let mut vars = HashMap::new();
    if let Ok(existing) = fs::read_to_string(&path) {
        vars = parse_env_file(&existing);
    }
    vars.insert("KEYMAP".to_string(), keymap.clone());

    // C (systemd-localed's write_vconsole_conf, shared by firstboot) prefixes
    // vconsole.conf with this fixed header; locale.conf gets none.
    let mut content = String::from(
        "# Written by systemd-localed(8) or systemd-firstboot(1), read by systemd-localed\n\
         # and systemd-vconsole-setup(8). Use localectl(1) to update this file.\n",
    );
    let mut keys: Vec<&String> = vars.keys().collect();
    keys.sort();
    for key in keys {
        content.push_str(&format!("{}={}\n", key, vars[key]));
    }
    fs::write(&path, content)?;
    eprintln!("Created {}.", path.display());
    Ok(true)
}

fn apply_timezone(root: &Path, settings: &Settings, force: bool) -> io::Result<bool> {
    let tz = match &settings.timezone {
        Some(t) => t,
        None => return Ok(false),
    };

    if !should_apply(root, "etc/localtime", force) {
        return Ok(false);
    }

    let link_path = root.join("etc/localtime");
    ensure_parent_dir(&link_path)?;

    // Remove existing symlink/file
    let _ = fs::remove_file(&link_path);

    let target = format!("../usr/share/zoneinfo/{}", tz);
    symlink(&target, &link_path)?;
    eprintln!("Created symlink {} → {}.", link_path.display(), target);
    Ok(true)
}

fn apply_hostname(root: &Path, settings: &Settings, force: bool) -> io::Result<bool> {
    let hostname = match &settings.hostname {
        Some(h) => h,
        None => return Ok(false),
    };

    if !should_apply(root, "etc/hostname", force) {
        return Ok(false);
    }

    let path = root.join("etc/hostname");
    ensure_parent_dir(&path)?;
    fs::write(&path, format!("{}\n", hostname))?;
    eprintln!("Created {}.", path.display());
    Ok(true)
}

fn apply_machine_id(root: &Path, settings: &Settings, force: bool) -> io::Result<bool> {
    let machine_id = match &settings.machine_id {
        Some(id) => id,
        None => return Ok(false),
    };

    if !should_apply(root, "etc/machine-id", force) {
        return Ok(false);
    }

    if !is_valid_machine_id(machine_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid machine ID: {}", machine_id),
        ));
    }

    let path = root.join("etc/machine-id");
    ensure_parent_dir(&path)?;
    fs::write(&path, format!("{}\n", machine_id))?;
    eprintln!("Created {}.", path.display());
    Ok(true)
}

fn apply_root_password(
    root: &Path,
    settings: &Settings,
    force: bool,
    delete: bool,
) -> io::Result<bool> {
    // Determine the hash to set
    let hash = if delete {
        String::new() // Empty password field = no password
    } else if let Some(ref pw) = settings.root_password {
        if settings.root_password_hashed == Some(true) {
            pw.clone()
        } else {
            hash_password(pw)
        }
    } else {
        return Ok(false);
    };

    // Leave an already-provisioned root account alone. Deleting a password is
    // an explicit request, so it is not gated on this.
    if !delete && !should_configure_root(root, force) {
        return Ok(false);
    }

    let shadow_path = root.join("etc/shadow");
    ensure_parent_dir(&shadow_path)?;

    // Read existing shadow file if it exists
    let existing = fs::read_to_string(&shadow_path).unwrap_or_default();
    let mut found_root = false;
    let mut lines: Vec<String> = Vec::new();

    for line in existing.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() >= 2 && fields[0] == "root" {
            found_root = true;
            // Replace password hash (field 1), preserve rest
            let mut new_fields: Vec<String> = fields.iter().map(|f| f.to_string()).collect();
            new_fields[1] = hash.clone();
            lines.push(new_fields.join(":"));
        } else {
            lines.push(line.to_string());
        }
    }

    if !found_root {
        // Create a minimal root entry
        // root:HASH:days_since_epoch:0:99999:7:::
        let days = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            / 86400;
        lines.push(format!("root:{}:{}:0:99999:7:::", hash, days));
    }

    let content = lines.join("\n") + "\n";
    fs::write(&shadow_path, content)?;

    // Set shadow file permissions to 0o640
    #[cfg(target_os = "linux")]
    {
        fs::set_permissions(&shadow_path, fs::Permissions::from_mode(0o640))?;
    }

    if delete {
        eprintln!("Deleted root password in {}.", shadow_path.display());
    } else {
        eprintln!("Set root password in {}.", shadow_path.display());
    }

    // A password in shadow is useless without the matching passwd entry, and
    // the account may not exist yet: TEST-74-AUX-UTILS.firstboot deletes both
    // files and then expects --root-password= alone to recreate them. Upstream
    // does the same, writing passwd and shadow together in
    // process_root_account() via write_root_passwd().
    write_root_passwd_entry(root, settings.root_shell.as_deref())?;

    Ok(true)
}

/// `struct passwd`'s password field meaning "look in the shadow file".
const PASSWORD_SEE_SHADOW: &str = "x";
/// What sysusers leaves in shadow to mean "firstboot should fill this in".
/// Note this is NOT the same as a locked password ("!*"): a root entry locked
/// with anything else, say "!test", counts as already configured.
const PASSWORD_UNPROVISIONED: &str = "!unprovisioned";

/// Whether the root account should be (re)configured, mirroring upstream's
/// should_configure() for passwd/shadow.
///
/// An already-provisioned root password must survive `--root-password=` unless
/// --force is given: TEST-74-AUX-UTILS.firstboot seeds `root:!test:` and then
/// requires it to still be there afterwards.
fn should_configure_root(root: &Path, force: bool) -> bool {
    if force {
        return true;
    }
    let Ok(passwd) = fs::read_to_string(root.join("etc/passwd")) else {
        return true; // missing
    };
    let mut saw_root = false;
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.first() != Some(&"root") {
            continue;
        }
        saw_root = true;
        // A root password held directly in passwd, rather than delegated to
        // shadow, means the account is already configured.
        if f.get(1).copied() != Some(PASSWORD_SEE_SHADOW) {
            return false;
        }
        break;
    }
    if !saw_root {
        return true;
    }
    let Ok(shadow) = fs::read_to_string(root.join("etc/shadow")) else {
        return true; // missing
    };
    for line in shadow.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.first() != Some(&"root") {
            continue;
        }
        return f.get(1).copied() == Some(PASSWORD_UNPROVISIONED);
    }
    true
}

/// Shell used for a freshly created root account when none was configured.
/// Upstream resolves this per-root via default_root_shell_at(); /bin/sh is the
/// portable fallback it ends up at when nothing else is configured.
const DEFAULT_ROOT_SHELL: &str = "/bin/sh";

/// Ensure `<root>/etc/passwd` has a root account, creating the file if needed.
///
/// An existing root line keeps its GECOS and home so only what firstboot owns
/// changes: the password field becomes "x" (the real hash lives in shadow) and
/// the shell is replaced when one was configured. Other accounts are left
/// untouched. Mirrors upstream's write_root_passwd().
fn write_root_passwd_entry(root: &Path, shell: Option<&str>) -> io::Result<()> {
    let passwd_path = root.join("etc/passwd");
    ensure_parent_dir(&passwd_path)?;

    let existing = fs::read_to_string(&passwd_path).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut found_root = false;

    for line in existing.lines() {
        if line.starts_with("root:") {
            found_root = true;
            // name:passwd:uid:gid:gecos:dir:shell
            let f: Vec<&str> = line.split(':').collect();
            let gecos = f.get(4).copied().unwrap_or("");
            let dir = f.get(5).copied().unwrap_or("/root");
            let old_shell = f.get(6).copied().unwrap_or(DEFAULT_ROOT_SHELL);
            let sh = shell.unwrap_or(old_shell);
            lines.push(format!("root:x:0:0:{gecos}:{dir}:{sh}"));
        } else {
            lines.push(line.to_string());
        }
    }

    if !found_root {
        let sh = shell.unwrap_or(DEFAULT_ROOT_SHELL);
        // Upstream puts root first when it creates the file.
        lines.insert(0, format!("root:x:0:0::/root:{sh}"));
    }

    let content = lines.join("\n") + "\n";
    fs::write(&passwd_path, content)?;
    #[cfg(target_os = "linux")]
    {
        fs::set_permissions(&passwd_path, fs::Permissions::from_mode(0o644))?;
    }
    Ok(())
}

fn apply_root_shell(root: &Path, settings: &Settings, force: bool) -> io::Result<bool> {
    let shell = match &settings.root_shell {
        Some(s) => s,
        None => return Ok(false),
    };

    // Gate on the same rule as the password, which is what upstream does:
    // process_root_account() decides once via should_configure() and then
    // writes the shell and the password together.
    //
    // This replaced a `!passwd_path.exists() && !force` early return that was
    // wrong in both directions. A missing passwd means the account still needs
    // creating, not that there is nothing to do, so `--root-shell=X` against a
    // tree with no passwd silently dropped the shell. But simply dropping that
    // check went too far the other way and let an already-configured root shell
    // be overwritten, which the test catches: it configures /bin/fooshell, then
    // runs firstboot again with --root-shell=shell-overwrite and requires
    // /bin/fooshell to survive.
    if !should_configure_root(root, force) {
        return Ok(false);
    }

    let passwd_path = root.join("etc/passwd");
    ensure_parent_dir(&passwd_path)?;

    let existing = fs::read_to_string(&passwd_path).unwrap_or_default();
    let mut found_root = false;
    let mut lines: Vec<String> = Vec::new();

    for line in existing.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() >= 7 && fields[0] == "root" {
            found_root = true;
            let mut new_fields: Vec<String> = fields.iter().map(|f| f.to_string()).collect();
            new_fields[6] = shell.clone();
            lines.push(new_fields.join(":"));
        } else {
            lines.push(line.to_string());
        }
    }

    let created_root = !found_root;
    if !found_root {
        // Create minimal root entry
        lines.push(format!("root:x:0:0:root:/root:{}", shell));
    }

    let content = lines.join("\n") + "\n";
    fs::write(&passwd_path, content)?;
    eprintln!("Set root shell to {} in {}.", shell, passwd_path.display());

    // An account created here has no password, and passwd only says "see
    // shadow". Leaving shadow without a matching entry would describe a root
    // account whose password is simply unknown, so give it an explicitly
    // locked, invalid one, as upstream does when no password was requested.
    if created_root {
        ensure_root_shadow_locked(root)?;
    }
    Ok(true)
}

/// A locked *and* invalid password, upstream's PASSWORD_LOCKED_AND_INVALID.
const PASSWORD_LOCKED_AND_INVALID: &str = "!*";

/// Give root a locked shadow entry, but only if it has none: an existing
/// password, provisioned or not, is left exactly as it is.
fn ensure_root_shadow_locked(root: &Path) -> io::Result<()> {
    let shadow_path = root.join("etc/shadow");
    let existing = fs::read_to_string(&shadow_path).unwrap_or_default();
    if existing
        .lines()
        .any(|l| l.split(':').next() == Some("root"))
    {
        return Ok(());
    }

    ensure_parent_dir(&shadow_path)?;
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86400;
    let mut lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();
    lines.push(format!("root:{PASSWORD_LOCKED_AND_INVALID}:{days}:0:99999:7:::"));
    fs::write(&shadow_path, lines.join("\n") + "\n")?;
    #[cfg(target_os = "linux")]
    {
        fs::set_permissions(&shadow_path, fs::Permissions::from_mode(0o640))?;
    }
    Ok(())
}

fn apply_kernel_cmdline(root: &Path, settings: &Settings, force: bool) -> io::Result<bool> {
    let cmdline = match &settings.kernel_cmdline {
        Some(c) => c,
        None => return Ok(false),
    };

    if !should_apply(root, "etc/kernel/cmdline", force) {
        return Ok(false);
    }

    let path = root.join("etc/kernel/cmdline");
    ensure_parent_dir(&path)?;
    fs::write(&path, format!("{}\n", cmdline))?;
    eprintln!("Created {}.", path.display());
    Ok(true)
}

// ---------------------------------------------------------------------------
// Reset settings
// ---------------------------------------------------------------------------

fn reset_file(root: &Path, relative: &str) -> io::Result<bool> {
    let path = root.join(relative);
    if path.exists() || path.is_symlink() {
        fs::remove_file(&path)?;
        eprintln!("Removed {}.", path.display());
        Ok(true)
    } else {
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ensure_parent_dir(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn print_usage() {
    eprintln!(
        "\
Usage: systemd-firstboot [OPTIONS...]

Initialize basic system settings on or before the first boot.

Options:
     --root=PATH              Operate on an alternate root filesystem
     --image=PATH             Operate on a disk image (not yet supported)
     --locale=LOCALE          Set system locale (LANG)
     --locale-messages=LOCALE Set message locale (LC_MESSAGES)
     --keymap=KEYMAP          Set console keymap
     --timezone=TIMEZONE      Set system timezone
     --hostname=NAME          Set system hostname
     --machine-id=ID          Set machine ID
     --root-password=PW       Set root password
     --root-password-hashed=H Set root password (pre-hashed)
     --root-password-file=F   Read root password from file
     --root-shell=SHELL       Set root login shell
     --kernel-command-line=CL Set kernel command line
     --setup-machine-id       Generate and set a new machine ID
     --delete-root-password   Delete root password
     --force                  Overwrite existing configuration files
     --prompt                 Prompt for all unconfigured settings
     --prompt-locale          Prompt for locale
     --prompt-keymap          Prompt for keymap
     --prompt-timezone        Prompt for timezone
     --prompt-hostname        Prompt for hostname
     --prompt-root-password   Prompt for root password
     --prompt-root-shell      Prompt for root shell
     --copy-locale            Copy locale from host
     --copy-keymap            Copy keymap from host
     --copy-timezone          Copy timezone from host
     --copy-root-password     Copy root password from host
     --copy-root-shell        Copy root shell from host
     --reset-locale           Remove locale configuration
     --reset-keymap           Remove keymap configuration
     --reset-timezone         Remove timezone configuration
     --reset-hostname         Remove hostname configuration
     --reset-machine-id       Remove machine ID
     --reset-root-password    Remove root password from shadow
     --reset-root-shell       Reset root shell in passwd
     --reset-kernel-cmdline   Remove kernel cmdline configuration
     --welcome=BOOL           Show welcome message (default: true)
  -h --help                   Show this help
     --version                Show version"
    );
}

fn print_welcome() {
    eprintln!();
    eprintln!("Welcome to your new installation!");
    eprintln!();
    eprintln!("Please configure basic system settings. These can be changed");
    eprintln!("at any time by editing the respective configuration files.");
    eprintln!();
}

// ---------------------------------------------------------------------------
// Main logic
// ---------------------------------------------------------------------------

fn run(argv: &[String]) -> Result<(), String> {
    let args = parse_args(argv)?;

    if args.image.is_some() {
        return Err("--image is not yet supported".to_string());
    }

    let root = &args.root;

    // Ensure root exists
    if !root.exists() {
        return Err(format!("Root directory does not exist: {}", root.display()));
    }

    // Process resets first
    let mut any_reset = false;
    if args.reset_locale {
        reset_file(root, "etc/locale.conf").map_err(|e| e.to_string())?;
        any_reset = true;
    }
    if args.reset_keymap {
        reset_file(root, "etc/vconsole.conf").map_err(|e| e.to_string())?;
        any_reset = true;
    }
    if args.reset_timezone {
        reset_file(root, "etc/localtime").map_err(|e| e.to_string())?;
        any_reset = true;
    }
    if args.reset_hostname {
        reset_file(root, "etc/hostname").map_err(|e| e.to_string())?;
        any_reset = true;
    }
    if args.reset_machine_id {
        reset_file(root, "etc/machine-id").map_err(|e| e.to_string())?;
        any_reset = true;
    }
    if args.reset_root_password {
        // Reset root password to locked
        let shadow_path = root.join("etc/shadow");
        if shadow_path.exists() {
            let content = fs::read_to_string(&shadow_path).map_err(|e| e.to_string())?;
            let mut lines: Vec<String> = Vec::new();
            for line in content.lines() {
                let fields: Vec<&str> = line.split(':').collect();
                if fields.len() >= 2 && fields[0] == "root" {
                    let mut new_fields: Vec<String> =
                        fields.iter().map(|f| f.to_string()).collect();
                    new_fields[1] = "!".to_string();
                    lines.push(new_fields.join(":"));
                } else {
                    lines.push(line.to_string());
                }
            }
            fs::write(&shadow_path, lines.join("\n") + "\n").map_err(|e| e.to_string())?;
            eprintln!("Reset root password in {}.", shadow_path.display());
        }
        any_reset = true;
    }
    if args.reset_root_shell {
        // Reset root shell to /bin/bash
        let passwd_path = root.join("etc/passwd");
        if passwd_path.exists() {
            let content = fs::read_to_string(&passwd_path).map_err(|e| e.to_string())?;
            let mut lines: Vec<String> = Vec::new();
            for line in content.lines() {
                let fields: Vec<&str> = line.split(':').collect();
                if fields.len() >= 7 && fields[0] == "root" {
                    let mut new_fields: Vec<String> =
                        fields.iter().map(|f| f.to_string()).collect();
                    new_fields[6] = "/bin/bash".to_string();
                    lines.push(new_fields.join(":"));
                } else {
                    lines.push(line.to_string());
                }
            }
            fs::write(&passwd_path, lines.join("\n") + "\n").map_err(|e| e.to_string())?;
            eprintln!("Reset root shell in {}.", passwd_path.display());
        }
        any_reset = true;
    }
    if args.reset_kernel_cmdline {
        reset_file(root, "etc/kernel/cmdline").map_err(|e| e.to_string())?;
        any_reset = true;
    }

    // If only resets were requested, we're done
    if any_reset
        && args.settings.locale.is_none()
        && args.settings.locale_messages.is_none()
        && args.settings.keymap.is_none()
        && args.settings.timezone.is_none()
        && args.settings.hostname.is_none()
        && args.settings.machine_id.is_none()
        && args.settings.root_password.is_none()
        && args.settings.root_shell.is_none()
        && args.settings.kernel_cmdline.is_none()
        && !args.prompt
        && !args.prompt_locale
        && !args.prompt_keymap
        && !args.prompt_timezone
        && !args.prompt_hostname
        && !args.prompt_root_password
        && !args.prompt_root_shell
        && !args.copy_locale
        && !args.copy_keymap
        && !args.copy_timezone
        && !args.copy_root_password
        && !args.copy_root_shell
        && !args.delete_root_password
    {
        return Ok(());
    }

    // Check if system was already booted.
    //
    // Only for the RUNNING system. Applied to an arbitrary --root tree this is
    // wrong and quietly destructive of intent: the check is "does <root>/etc/
    // machine-id hold a real id", so as soon as anything sets a machine-id in
    // that tree, every later firstboot call against it silently does nothing.
    // TEST-74-AUX-UTILS.firstboot walks straight into that, setting
    // --machine-id= and then finding --root-password= ignored.
    //
    // Upstream has no such blanket guard in firstboot itself; first-boot-ness
    // gates the SERVICE via ConditionFirstBoot=, while the tool applies what it
    // was asked for and each step decides on its own file.
    if root == Path::new("/") && system_already_booted(root) && !args.force {
        eprintln!("System already booted, skipping firstboot configuration.");
        eprintln!("Use --force to override.");
        return Ok(());
    }

    let mut settings = args.settings.clone();

    // Load credentials
    load_settings_from_credentials(&mut settings);

    // Copy from host.
    //
    // For locale and keymap this is a VERBATIM file copy, as upstream does with
    // copy_file_atomic_at(). Reconstructing the file from the LANG/LC_MESSAGES
    // or KEYMAP values it happens to parse would drop every other variable, and
    // any comments and formatting with them; the test diffs the copy against
    // the host file, so it has to match byte for byte. Parsing into settings
    // remains the fallback for when the host has no such file.
    if args.copy_locale
        && !copy_host_file_verbatim(root, "etc/locale.conf", args.force)
            .map_err(|e| e.to_string())?
    {
        copy_locale_from_host(&mut settings);
    }
    if args.copy_keymap
        && !copy_host_file_verbatim(root, "etc/vconsole.conf", args.force)
            .map_err(|e| e.to_string())?
    {
        copy_keymap_from_host(&mut settings);
    }
    if args.copy_timezone {
        copy_timezone_from_host(&mut settings);
    }
    if args.copy_root_password {
        copy_root_password_from_host(&mut settings);
    }
    if args.copy_root_shell {
        copy_root_shell_from_host(&mut settings);
    }

    // Interactive prompts
    let prompting = args.prompt
        || args.prompt_locale
        || args.prompt_keymap
        || args.prompt_timezone
        || args.prompt_hostname
        || args.prompt_root_password
        || args.prompt_root_shell;

    if prompting && args.welcome {
        print_welcome();
    }

    if (args.prompt || args.prompt_locale) && settings.locale.is_none() {
        settings.locale = prompt_value("System locale (LANG)", Some("C.UTF-8"));
    }

    // Upstream asks for LC_MESSAGES straight after LANG, so --prompt-locale
    // consumes TWO answers, not one. Only asking for LANG left the second line
    // of input unread and LC_MESSAGES unset.
    if (args.prompt || args.prompt_locale) && settings.locale_messages.is_none() {
        settings.locale_messages = prompt_value("System message locale (LC_MESSAGES)", None);
    }

    if (args.prompt || args.prompt_keymap) && settings.keymap.is_none() {
        settings.keymap = prompt_value("Console keymap", Some("us"));
    }

    if (args.prompt || args.prompt_timezone) && settings.timezone.is_none() {
        settings.timezone = prompt_value("Timezone", Some("UTC"));
    }

    if (args.prompt || args.prompt_hostname) && settings.hostname.is_none() {
        loop {
            let input = prompt_value("Hostname", Some("localhost"));
            match input {
                Some(ref h) if is_valid_hostname(h) => {
                    settings.hostname = input;
                    break;
                }
                Some(ref h) => {
                    eprintln!("Invalid hostname: {}", h);
                }
                None => break,
            }
        }
    }

    if (args.prompt || args.prompt_root_password)
        && settings.root_password.is_none()
        && !args.delete_root_password
    {
        loop {
            let pw1 = prompt_password("Root password");
            if pw1.is_none() {
                break;
            }
            let pw2 = prompt_password("Repeat root password");
            if pw1 == pw2 {
                settings.root_password = pw1;
                settings.root_password_hashed = Some(false);
                break;
            } else {
                eprintln!("Passwords do not match, please try again.");
            }
        }
    }

    if (args.prompt || args.prompt_root_shell) && settings.root_shell.is_none() {
        settings.root_shell = prompt_value("Root shell", Some("/bin/bash"));
    }

    // Apply settings
    let mut any_applied = false;

    // Being asked for a root password and declining to give one is still an
    // answer: upstream creates a LOCKED root account rather than leaving the
    // tree with no root entry at all. Without this, `--prompt-root-password
    // </dev/null` succeeded while writing nothing.
    if (args.prompt || args.prompt_root_password)
        && settings.root_password.is_none()
        && !args.delete_root_password
        && should_configure_root(root, args.force)
    {
        write_root_passwd_entry(root, settings.root_shell.as_deref())
            .map_err(|e| e.to_string())?;
        ensure_root_shadow_locked(root).map_err(|e| e.to_string())?;
        any_applied = true;
    }

    if apply_locale(root, &settings, args.force).map_err(|e| e.to_string())? {
        any_applied = true;
    }
    if apply_keymap(root, &settings, args.force).map_err(|e| e.to_string())? {
        any_applied = true;
    }
    if apply_timezone(root, &settings, args.force).map_err(|e| e.to_string())? {
        any_applied = true;
    }
    if apply_hostname(root, &settings, args.force).map_err(|e| e.to_string())? {
        any_applied = true;
    }
    if apply_machine_id(root, &settings, args.force).map_err(|e| e.to_string())? {
        any_applied = true;
    }
    if apply_root_password(root, &settings, args.force, args.delete_root_password)
        .map_err(|e| e.to_string())?
    {
        any_applied = true;
    }
    if apply_root_shell(root, &settings, args.force).map_err(|e| e.to_string())? {
        any_applied = true;
    }
    if apply_kernel_cmdline(root, &settings, args.force).map_err(|e| e.to_string())? {
        any_applied = true;
    }

    if !any_applied && !any_reset && !prompting {
        eprintln!("No settings to apply.");
    }

    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("systemd-firstboot: {}", e);
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

    fn setup_root(root: &Path) {
        fs::create_dir_all(root.join("etc")).unwrap();
    }

    // -----------------------------------------------------------------------
    // parse_args tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_args_empty() {
        let args = parse_args(&[]).unwrap();
        assert_eq!(args.root, PathBuf::from("/"));
        assert!(!args.force);
        assert!(!args.prompt);
        assert!(args.settings.locale.is_none());
    }

    #[test]
    fn test_parse_args_locale() {
        let args = parse_args(&["--locale=en_US.UTF-8".to_string()]).unwrap();
        assert_eq!(args.settings.locale, Some("en_US.UTF-8".to_string()));
    }

    #[test]
    fn test_parse_args_locale_separate() {
        let args = parse_args(&["--locale".to_string(), "en_US.UTF-8".to_string()]).unwrap();
        assert_eq!(args.settings.locale, Some("en_US.UTF-8".to_string()));
    }

    #[test]
    fn test_parse_args_root() {
        let args = parse_args(&["--root=/sysroot".to_string()]).unwrap();
        assert_eq!(args.root, PathBuf::from("/sysroot"));
    }

    #[test]
    fn test_parse_args_force() {
        let args = parse_args(&["--force".to_string()]).unwrap();
        assert!(args.force);
    }

    #[test]
    fn test_parse_args_prompt() {
        let args = parse_args(&["--prompt".to_string()]).unwrap();
        assert!(args.prompt);
    }

    #[test]
    fn test_parse_args_prompt_locale() {
        let args = parse_args(&["--prompt-locale".to_string()]).unwrap();
        assert!(args.prompt_locale);
    }

    #[test]
    fn test_parse_args_copy_locale() {
        let args = parse_args(&["--copy-locale".to_string()]).unwrap();
        assert!(args.copy_locale);
    }

    #[test]
    fn test_parse_args_reset_locale() {
        let args = parse_args(&["--reset-locale".to_string()]).unwrap();
        assert!(args.reset_locale);
    }

    #[test]
    fn test_parse_args_hostname() {
        let args = parse_args(&["--hostname=myhost".to_string()]).unwrap();
        assert_eq!(args.settings.hostname, Some("myhost".to_string()));
    }

    #[test]
    fn test_parse_args_timezone() {
        let args = parse_args(&["--timezone=Europe/Berlin".to_string()]).unwrap();
        assert_eq!(args.settings.timezone, Some("Europe/Berlin".to_string()));
    }

    #[test]
    fn test_parse_args_machine_id() {
        let args =
            parse_args(&["--machine-id=0123456789abcdef0123456789abcdef".to_string()]).unwrap();
        assert_eq!(
            args.settings.machine_id,
            Some("0123456789abcdef0123456789abcdef".to_string())
        );
    }

    #[test]
    fn test_parse_args_keymap() {
        let args = parse_args(&["--keymap=de".to_string()]).unwrap();
        assert_eq!(args.settings.keymap, Some("de".to_string()));
    }

    #[test]
    fn test_parse_args_root_password() {
        let args = parse_args(&["--root-password=secret".to_string()]).unwrap();
        assert_eq!(args.settings.root_password, Some("secret".to_string()));
        assert_eq!(args.settings.root_password_hashed, Some(false));
    }

    #[test]
    fn test_parse_args_root_password_hashed() {
        let args = parse_args(&["--root-password-hashed=$6$salt$hash".to_string()]).unwrap();
        assert_eq!(
            args.settings.root_password,
            Some("$6$salt$hash".to_string())
        );
        assert_eq!(args.settings.root_password_hashed, Some(true));
    }

    #[test]
    fn test_parse_args_root_shell() {
        let args = parse_args(&["--root-shell=/bin/zsh".to_string()]).unwrap();
        assert_eq!(args.settings.root_shell, Some("/bin/zsh".to_string()));
    }

    #[test]
    fn test_parse_args_kernel_cmdline() {
        let args = parse_args(&["--kernel-command-line=quiet splash".to_string()]).unwrap();
        assert_eq!(
            args.settings.kernel_cmdline,
            Some("quiet splash".to_string())
        );
    }

    #[test]
    fn test_parse_args_delete_root_password() {
        let args = parse_args(&["--delete-root-password".to_string()]).unwrap();
        assert!(args.delete_root_password);
    }

    #[test]
    fn test_parse_args_welcome_false() {
        let args = parse_args(&["--welcome=false".to_string()]).unwrap();
        assert!(!args.welcome);
    }

    #[test]
    fn test_parse_args_welcome_true() {
        let args = parse_args(&["--welcome=yes".to_string()]).unwrap();
        assert!(args.welcome);
    }

    #[test]
    fn test_parse_args_setup_machine_id() {
        let args = parse_args(&["--setup-machine-id".to_string()]).unwrap();
        assert!(args.settings.machine_id.is_some());
        assert!(is_valid_machine_id(
            args.settings.machine_id.as_ref().unwrap()
        ));
    }

    #[test]
    fn test_parse_args_multiple_options() {
        let args = parse_args(&[
            "--locale=en_US.UTF-8".to_string(),
            "--hostname=myhost".to_string(),
            "--timezone=UTC".to_string(),
            "--force".to_string(),
        ])
        .unwrap();
        assert_eq!(args.settings.locale, Some("en_US.UTF-8".to_string()));
        assert_eq!(args.settings.hostname, Some("myhost".to_string()));
        assert_eq!(args.settings.timezone, Some("UTC".to_string()));
        assert!(args.force);
    }

    #[test]
    fn test_parse_args_unknown_option() {
        let result = parse_args(&["--nonexistent".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_missing_value() {
        let result = parse_args(&["--locale".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_all_resets() {
        let args = parse_args(&[
            "--reset-locale".to_string(),
            "--reset-keymap".to_string(),
            "--reset-timezone".to_string(),
            "--reset-hostname".to_string(),
            "--reset-machine-id".to_string(),
            "--reset-root-password".to_string(),
            "--reset-root-shell".to_string(),
            "--reset-kernel-cmdline".to_string(),
        ])
        .unwrap();
        assert!(args.reset_locale);
        assert!(args.reset_keymap);
        assert!(args.reset_timezone);
        assert!(args.reset_hostname);
        assert!(args.reset_machine_id);
        assert!(args.reset_root_password);
        assert!(args.reset_root_shell);
        assert!(args.reset_kernel_cmdline);
    }

    #[test]
    fn test_parse_args_all_copies() {
        let args = parse_args(&[
            "--copy-locale".to_string(),
            "--copy-keymap".to_string(),
            "--copy-timezone".to_string(),
            "--copy-root-password".to_string(),
            "--copy-root-shell".to_string(),
        ])
        .unwrap();
        assert!(args.copy_locale);
        assert!(args.copy_keymap);
        assert!(args.copy_timezone);
        assert!(args.copy_root_password);
        assert!(args.copy_root_shell);
    }

    #[test]
    fn test_parse_args_all_prompts() {
        let args = parse_args(&[
            "--prompt-locale".to_string(),
            "--prompt-keymap".to_string(),
            "--prompt-timezone".to_string(),
            "--prompt-hostname".to_string(),
            "--prompt-root-password".to_string(),
            "--prompt-root-shell".to_string(),
        ])
        .unwrap();
        assert!(args.prompt_locale);
        assert!(args.prompt_keymap);
        assert!(args.prompt_timezone);
        assert!(args.prompt_hostname);
        assert!(args.prompt_root_password);
        assert!(args.prompt_root_shell);
    }

    // -----------------------------------------------------------------------
    // parse_bool tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_bool() {
        assert!(parse_bool("1"));
        assert!(parse_bool("yes"));
        assert!(parse_bool("true"));
        assert!(parse_bool("on"));
        assert!(parse_bool("y"));
        assert!(parse_bool("YES"));
        assert!(parse_bool("True"));
        assert!(!parse_bool("0"));
        assert!(!parse_bool("no"));
        assert!(!parse_bool("false"));
        assert!(!parse_bool("off"));
        assert!(!parse_bool(""));
    }

    // -----------------------------------------------------------------------
    // Machine ID tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_generate_machine_id_valid() {
        let id = generate_machine_id();
        assert!(is_valid_machine_id(&id), "Generated ID '{}' is invalid", id);
    }

    #[test]
    fn test_generate_machine_id_uniqueness() {
        let id1 = generate_machine_id();
        let id2 = generate_machine_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_is_valid_machine_id_valid() {
        assert!(is_valid_machine_id("0123456789abcdef0123456789abcdef"));
        assert!(is_valid_machine_id("AABBCCDD00112233AABBCCDD00112233"));
    }

    #[test]
    fn test_is_valid_machine_id_invalid() {
        assert!(!is_valid_machine_id(""));
        assert!(!is_valid_machine_id("too-short"));
        assert!(!is_valid_machine_id("0123456789abcdef0123456789abcdeg")); // 'g'
        assert!(!is_valid_machine_id("0123456789abcdef0123456789abcdef0")); // too long
        assert!(!is_valid_machine_id("0123456789abcdef0123456789abcde")); // too short
    }

    // -----------------------------------------------------------------------
    // Hostname validation tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_valid_hostname_valid() {
        assert!(is_valid_hostname("myhost"));
        assert!(is_valid_hostname("my-host"));
        assert!(is_valid_hostname("my.host.com"));
        assert!(is_valid_hostname("host123"));
        assert!(is_valid_hostname("A"));
    }

    #[test]
    fn test_is_valid_hostname_invalid() {
        assert!(!is_valid_hostname(""));
        assert!(!is_valid_hostname("-myhost"));
        assert!(!is_valid_hostname("myhost-"));
        assert!(!is_valid_hostname(".myhost"));
        assert!(!is_valid_hostname("myhost."));
        assert!(!is_valid_hostname("my host")); // space
        assert!(!is_valid_hostname("my_host")); // underscore
        assert!(!is_valid_hostname(
            &"a".repeat(65) // too long
        ));
    }

    #[test]
    fn test_is_valid_hostname_exactly_64() {
        assert!(is_valid_hostname(&"a".repeat(64)));
    }

    // -----------------------------------------------------------------------
    // Environment file parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_env_file_basic() {
        let content = "LANG=en_US.UTF-8\nLC_MESSAGES=C\n";
        let vars = parse_env_file(content);
        assert_eq!(vars.get("LANG"), Some(&"en_US.UTF-8".to_string()));
        assert_eq!(vars.get("LC_MESSAGES"), Some(&"C".to_string()));
    }

    #[test]
    fn test_parse_env_file_quoted() {
        let content = "LANG=\"en_US.UTF-8\"\nLC_MESSAGES='C'\n";
        let vars = parse_env_file(content);
        assert_eq!(vars.get("LANG"), Some(&"en_US.UTF-8".to_string()));
        assert_eq!(vars.get("LC_MESSAGES"), Some(&"C".to_string()));
    }

    #[test]
    fn test_parse_env_file_comments_and_blanks() {
        let content = "# Comment\n\nLANG=C\n   # Another comment\n";
        let vars = parse_env_file(content);
        assert_eq!(vars.len(), 1);
        assert_eq!(vars.get("LANG"), Some(&"C".to_string()));
    }

    #[test]
    fn test_parse_env_file_empty() {
        let vars = parse_env_file("");
        assert!(vars.is_empty());
    }

    #[test]
    fn test_parse_env_file_whitespace() {
        let content = "  LANG = en_US.UTF-8  \n";
        let vars = parse_env_file(content);
        assert_eq!(vars.get("LANG"), Some(&"en_US.UTF-8".to_string()));
    }

    // -----------------------------------------------------------------------
    // format_env_file tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_env_file_simple() {
        let result = format_env_file(&[("LANG", "C")]);
        assert_eq!(result, "LANG=C\n");
    }

    #[test]
    fn test_format_env_file_with_spaces() {
        let result = format_env_file(&[("KEY", "value with spaces")]);
        assert_eq!(result, "KEY=\"value with spaces\"\n");
    }

    #[test]
    fn test_format_env_file_multiple() {
        let result = format_env_file(&[("LANG", "C"), ("LC_MESSAGES", "en_US.UTF-8")]);
        assert_eq!(result, "LANG=C\nLC_MESSAGES=en_US.UTF-8\n");
    }

    // -----------------------------------------------------------------------
    // apply_locale tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_apply_locale_none() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings::default();
        let result = apply_locale(tmp.path(), &settings, false).unwrap();
        assert!(!result);
        assert!(!tmp.path().join("etc/locale.conf").exists());
    }

    #[test]
    fn test_apply_locale_basic() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings {
            locale: Some("en_US.UTF-8".to_string()),
            ..Default::default()
        };
        let result = apply_locale(tmp.path(), &settings, false).unwrap();
        assert!(result);
        let content = fs::read_to_string(tmp.path().join("etc/locale.conf")).unwrap();
        assert!(content.contains("LANG=en_US.UTF-8"));
    }

    #[test]
    fn test_apply_locale_with_messages() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings {
            locale: Some("en_US.UTF-8".to_string()),
            locale_messages: Some("C".to_string()),
            ..Default::default()
        };
        let result = apply_locale(tmp.path(), &settings, false).unwrap();
        assert!(result);
        let content = fs::read_to_string(tmp.path().join("etc/locale.conf")).unwrap();
        assert!(content.contains("LANG=en_US.UTF-8"));
        assert!(content.contains("LC_MESSAGES=C"));
    }

    #[test]
    fn test_apply_locale_no_overwrite() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(tmp.path().join("etc/locale.conf"), "LANG=C\n").unwrap();
        let settings = Settings {
            locale: Some("en_US.UTF-8".to_string()),
            ..Default::default()
        };
        let result = apply_locale(tmp.path(), &settings, false).unwrap();
        assert!(!result);
        let content = fs::read_to_string(tmp.path().join("etc/locale.conf")).unwrap();
        assert_eq!(content, "LANG=C\n"); // unchanged
    }

    #[test]
    fn test_apply_locale_force_overwrite() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(tmp.path().join("etc/locale.conf"), "LANG=C\n").unwrap();
        let settings = Settings {
            locale: Some("en_US.UTF-8".to_string()),
            ..Default::default()
        };
        let result = apply_locale(tmp.path(), &settings, true).unwrap();
        assert!(result);
        let content = fs::read_to_string(tmp.path().join("etc/locale.conf")).unwrap();
        assert!(content.contains("LANG=en_US.UTF-8"));
    }

    // -----------------------------------------------------------------------
    // apply_keymap tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_apply_keymap_none() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings::default();
        let result = apply_keymap(tmp.path(), &settings, false).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_apply_keymap_basic() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings {
            keymap: Some("de".to_string()),
            ..Default::default()
        };
        let result = apply_keymap(tmp.path(), &settings, false).unwrap();
        assert!(result);
        let content = fs::read_to_string(tmp.path().join("etc/vconsole.conf")).unwrap();
        assert!(content.contains("KEYMAP=de"));
        // C prefixes vconsole.conf with the systemd-localed header; match it.
        assert!(
            content.starts_with(
                "# Written by systemd-localed(8) or systemd-firstboot(1), read by systemd-localed\n"
            ),
            "{content}"
        );
    }

    #[test]
    fn test_apply_keymap_preserves_existing() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        // The file is non-empty so should_apply returns false
        fs::write(
            tmp.path().join("etc/vconsole.conf"),
            "FONT=Lat2-Terminus16\n",
        )
        .unwrap();
        let settings = Settings {
            keymap: Some("de".to_string()),
            ..Default::default()
        };
        // Without force, won't overwrite
        let result = apply_keymap(tmp.path(), &settings, false).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_apply_keymap_force_merges() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(
            tmp.path().join("etc/vconsole.conf"),
            "FONT=Lat2-Terminus16\n",
        )
        .unwrap();
        let settings = Settings {
            keymap: Some("de".to_string()),
            ..Default::default()
        };
        let result = apply_keymap(tmp.path(), &settings, true).unwrap();
        assert!(result);
        let content = fs::read_to_string(tmp.path().join("etc/vconsole.conf")).unwrap();
        assert!(content.contains("KEYMAP=de"));
        assert!(content.contains("FONT=Lat2-Terminus16"));
    }

    // -----------------------------------------------------------------------
    // apply_timezone tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_apply_timezone_none() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings::default();
        let result = apply_timezone(tmp.path(), &settings, false).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_apply_timezone_basic() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings {
            timezone: Some("Europe/Berlin".to_string()),
            ..Default::default()
        };
        let result = apply_timezone(tmp.path(), &settings, false).unwrap();
        assert!(result);
        let link = fs::read_link(tmp.path().join("etc/localtime")).unwrap();
        assert_eq!(
            link.to_string_lossy(),
            "../usr/share/zoneinfo/Europe/Berlin"
        );
    }

    #[test]
    fn test_apply_timezone_no_overwrite_symlink() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let link_path = tmp.path().join("etc/localtime");
        symlink("../usr/share/zoneinfo/UTC", &link_path).unwrap();
        let settings = Settings {
            timezone: Some("Europe/Berlin".to_string()),
            ..Default::default()
        };
        let result = apply_timezone(tmp.path(), &settings, false).unwrap();
        assert!(!result);
        let link = fs::read_link(&link_path).unwrap();
        assert_eq!(link.to_string_lossy(), "../usr/share/zoneinfo/UTC");
    }

    #[test]
    fn test_apply_timezone_force_overwrite() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let link_path = tmp.path().join("etc/localtime");
        symlink("../usr/share/zoneinfo/UTC", &link_path).unwrap();
        let settings = Settings {
            timezone: Some("Europe/Berlin".to_string()),
            ..Default::default()
        };
        let result = apply_timezone(tmp.path(), &settings, true).unwrap();
        assert!(result);
        let link = fs::read_link(&link_path).unwrap();
        assert_eq!(
            link.to_string_lossy(),
            "../usr/share/zoneinfo/Europe/Berlin"
        );
    }

    // -----------------------------------------------------------------------
    // apply_hostname tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_apply_hostname_none() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings::default();
        let result = apply_hostname(tmp.path(), &settings, false).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_apply_hostname_basic() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings {
            hostname: Some("myhost".to_string()),
            ..Default::default()
        };
        let result = apply_hostname(tmp.path(), &settings, false).unwrap();
        assert!(result);
        let content = fs::read_to_string(tmp.path().join("etc/hostname")).unwrap();
        assert_eq!(content, "myhost\n");
    }

    #[test]
    fn test_apply_hostname_no_overwrite() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(tmp.path().join("etc/hostname"), "existing\n").unwrap();
        let settings = Settings {
            hostname: Some("myhost".to_string()),
            ..Default::default()
        };
        let result = apply_hostname(tmp.path(), &settings, false).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_apply_hostname_force() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(tmp.path().join("etc/hostname"), "existing\n").unwrap();
        let settings = Settings {
            hostname: Some("myhost".to_string()),
            ..Default::default()
        };
        let result = apply_hostname(tmp.path(), &settings, true).unwrap();
        assert!(result);
        let content = fs::read_to_string(tmp.path().join("etc/hostname")).unwrap();
        assert_eq!(content, "myhost\n");
    }

    // -----------------------------------------------------------------------
    // apply_machine_id tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_apply_machine_id_none() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings::default();
        let result = apply_machine_id(tmp.path(), &settings, false).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_apply_machine_id_valid() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings {
            machine_id: Some("0123456789abcdef0123456789abcdef".to_string()),
            ..Default::default()
        };
        let result = apply_machine_id(tmp.path(), &settings, false).unwrap();
        assert!(result);
        let content = fs::read_to_string(tmp.path().join("etc/machine-id")).unwrap();
        assert_eq!(content, "0123456789abcdef0123456789abcdef\n");
    }

    #[test]
    fn test_apply_machine_id_invalid() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings {
            machine_id: Some("invalid".to_string()),
            ..Default::default()
        };
        let result = apply_machine_id(tmp.path(), &settings, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_apply_machine_id_no_overwrite() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(
            tmp.path().join("etc/machine-id"),
            "aabbccddaabbccddaabbccddaabbccdd\n",
        )
        .unwrap();
        let settings = Settings {
            machine_id: Some("0123456789abcdef0123456789abcdef".to_string()),
            ..Default::default()
        };
        let result = apply_machine_id(tmp.path(), &settings, false).unwrap();
        assert!(!result);
    }

    // -----------------------------------------------------------------------
    // apply_root_password tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_apply_root_password_none() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings::default();
        let result = apply_root_password(tmp.path(), &settings, false, false).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_apply_root_password_hashed() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings {
            root_password: Some("$6$salt$hashvalue".to_string()),
            root_password_hashed: Some(true),
            ..Default::default()
        };
        let result = apply_root_password(tmp.path(), &settings, false, false).unwrap();
        assert!(result);
        let content = fs::read_to_string(tmp.path().join("etc/shadow")).unwrap();
        assert!(content.contains("root:$6$salt$hashvalue:"));
    }

    #[test]
    fn test_apply_root_password_updates_existing() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(
            tmp.path().join("etc/shadow"),
            "root:!:19000:0:99999:7:::\nnobody:*:19000:0:99999:7:::\n",
        )
        .unwrap();
        let settings = Settings {
            root_password: Some("$6$new$newhash".to_string()),
            root_password_hashed: Some(true),
            ..Default::default()
        };
        let result = apply_root_password(tmp.path(), &settings, true, false).unwrap();
        assert!(result);
        let content = fs::read_to_string(tmp.path().join("etc/shadow")).unwrap();
        assert!(content.contains("root:$6$new$newhash:"));
        assert!(content.contains("nobody:*:"));
    }

    #[test]
    fn test_apply_root_password_delete() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(
            tmp.path().join("etc/shadow"),
            "root:$6$existing$hash:19000:0:99999:7:::\n",
        )
        .unwrap();
        let settings = Settings::default();
        let result = apply_root_password(tmp.path(), &settings, true, true).unwrap();
        assert!(result);
        let content = fs::read_to_string(tmp.path().join("etc/shadow")).unwrap();
        assert!(content.contains("root::"));
    }

    // -----------------------------------------------------------------------
    // apply_root_shell tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_apply_root_shell_none() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings::default();
        let result = apply_root_shell(tmp.path(), &settings, false).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_apply_root_shell_updates_existing() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(
            tmp.path().join("etc/passwd"),
            "root:x:0:0:root:/root:/bin/bash\nnobody:x:65534:65534:Nobody:/:/usr/bin/nologin\n",
        )
        .unwrap();
        let settings = Settings {
            root_shell: Some("/bin/zsh".to_string()),
            ..Default::default()
        };
        let result = apply_root_shell(tmp.path(), &settings, true).unwrap();
        assert!(result);
        let content = fs::read_to_string(tmp.path().join("etc/passwd")).unwrap();
        assert!(content.contains("root:x:0:0:root:/root:/bin/zsh"));
        assert!(content.contains("nobody:x:65534:65534:Nobody:/:/usr/bin/nologin"));
    }

    #[test]
    fn test_apply_root_shell_creates_entry() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings {
            root_shell: Some("/bin/zsh".to_string()),
            ..Default::default()
        };
        // Create an empty passwd file
        fs::write(tmp.path().join("etc/passwd"), "").unwrap();
        let result = apply_root_shell(tmp.path(), &settings, true).unwrap();
        assert!(result);
        let content = fs::read_to_string(tmp.path().join("etc/passwd")).unwrap();
        assert!(content.contains("root:x:0:0:root:/root:/bin/zsh"));
    }

    // -----------------------------------------------------------------------
    // apply_kernel_cmdline tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_apply_kernel_cmdline_none() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings::default();
        let result = apply_kernel_cmdline(tmp.path(), &settings, false).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_apply_kernel_cmdline_basic() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let settings = Settings {
            kernel_cmdline: Some("quiet splash".to_string()),
            ..Default::default()
        };
        let result = apply_kernel_cmdline(tmp.path(), &settings, false).unwrap();
        assert!(result);
        let content = fs::read_to_string(tmp.path().join("etc/kernel/cmdline")).unwrap();
        assert_eq!(content, "quiet splash\n");
    }

    #[test]
    fn test_apply_kernel_cmdline_creates_parent_dir() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        // Don't create etc/kernel/ — should be auto-created
        let settings = Settings {
            kernel_cmdline: Some("root=/dev/sda1".to_string()),
            ..Default::default()
        };
        let result = apply_kernel_cmdline(tmp.path(), &settings, false).unwrap();
        assert!(result);
        assert!(tmp.path().join("etc/kernel/cmdline").exists());
    }

    // -----------------------------------------------------------------------
    // reset_file tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_reset_file_exists() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(tmp.path().join("etc/hostname"), "myhost\n").unwrap();
        let result = reset_file(tmp.path(), "etc/hostname").unwrap();
        assert!(result);
        assert!(!tmp.path().join("etc/hostname").exists());
    }

    #[test]
    fn test_reset_file_not_exists() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let result = reset_file(tmp.path(), "etc/hostname").unwrap();
        assert!(!result);
    }

    #[test]
    fn test_reset_file_symlink() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        symlink(
            "../usr/share/zoneinfo/UTC",
            tmp.path().join("etc/localtime"),
        )
        .unwrap();
        let result = reset_file(tmp.path(), "etc/localtime").unwrap();
        assert!(result);
        assert!(!tmp.path().join("etc/localtime").exists());
    }

    // -----------------------------------------------------------------------
    // system_already_booted tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_system_already_booted_no_file() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        assert!(!system_already_booted(tmp.path()));
    }

    #[test]
    fn test_system_already_booted_empty() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(tmp.path().join("etc/machine-id"), "").unwrap();
        assert!(!system_already_booted(tmp.path()));
    }

    #[test]
    fn test_system_already_booted_uninitialized() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(tmp.path().join("etc/machine-id"), "uninitialized\n").unwrap();
        assert!(!system_already_booted(tmp.path()));
    }

    #[test]
    fn test_system_already_booted_valid() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(
            tmp.path().join("etc/machine-id"),
            "0123456789abcdef0123456789abcdef\n",
        )
        .unwrap();
        assert!(system_already_booted(tmp.path()));
    }

    // -----------------------------------------------------------------------
    // target_exists / should_apply tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_target_exists_no_file() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        assert!(!target_exists(tmp.path(), "etc/hostname"));
    }

    #[test]
    fn test_target_exists_empty_file() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(tmp.path().join("etc/hostname"), "").unwrap();
        assert!(!target_exists(tmp.path(), "etc/hostname"));
    }

    #[test]
    fn test_target_exists_nonempty_file() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(tmp.path().join("etc/hostname"), "myhost\n").unwrap();
        assert!(target_exists(tmp.path(), "etc/hostname"));
    }

    #[test]
    fn test_target_exists_symlink() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        symlink(
            "../usr/share/zoneinfo/UTC",
            tmp.path().join("etc/localtime"),
        )
        .unwrap();
        assert!(target_exists(tmp.path(), "etc/localtime"));
    }

    #[test]
    fn test_should_apply_no_file() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        assert!(should_apply(tmp.path(), "etc/hostname", false));
    }

    #[test]
    fn test_should_apply_file_exists() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(tmp.path().join("etc/hostname"), "myhost\n").unwrap();
        assert!(!should_apply(tmp.path(), "etc/hostname", false));
    }

    #[test]
    fn test_should_apply_force() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(tmp.path().join("etc/hostname"), "myhost\n").unwrap();
        assert!(should_apply(tmp.path(), "etc/hostname", true));
    }

    // -----------------------------------------------------------------------
    // read_root_password_hash tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_read_root_password_hash_valid() {
        let tmp = temp_dir();
        let shadow = tmp.path().join("shadow");
        fs::write(
            &shadow,
            "root:$6$salt$hash:19000:0:99999:7:::\nnobody:*:19000:0:99999:7:::\n",
        )
        .unwrap();
        let hash = read_root_password_hash(shadow.to_str().unwrap());
        assert_eq!(hash, Some("$6$salt$hash".to_string()));
    }

    #[test]
    fn test_read_root_password_hash_locked() {
        let tmp = temp_dir();
        let shadow = tmp.path().join("shadow");
        fs::write(&shadow, "root:!:19000:0:99999:7:::\n").unwrap();
        let hash = read_root_password_hash(shadow.to_str().unwrap());
        assert!(hash.is_none());
    }

    #[test]
    fn test_read_root_password_hash_x() {
        let tmp = temp_dir();
        let shadow = tmp.path().join("shadow");
        fs::write(&shadow, "root:x:19000:0:99999:7:::\n").unwrap();
        let hash = read_root_password_hash(shadow.to_str().unwrap());
        assert!(hash.is_none());
    }

    #[test]
    fn test_read_root_password_hash_empty() {
        let tmp = temp_dir();
        let shadow = tmp.path().join("shadow");
        fs::write(&shadow, "root::19000:0:99999:7:::\n").unwrap();
        let hash = read_root_password_hash(shadow.to_str().unwrap());
        assert!(hash.is_none());
    }

    #[test]
    fn test_read_root_password_hash_missing_file() {
        let hash = read_root_password_hash("/nonexistent/shadow");
        assert!(hash.is_none());
    }

    // -----------------------------------------------------------------------
    // read_root_shell tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_read_root_shell_basic() {
        let tmp = temp_dir();
        let passwd = tmp.path().join("passwd");
        fs::write(
            &passwd,
            "root:x:0:0:root:/root:/bin/bash\nnobody:x:65534:65534:Nobody:/:/usr/bin/nologin\n",
        )
        .unwrap();
        let shell = read_root_shell(passwd.to_str().unwrap());
        assert_eq!(shell, Some("/bin/bash".to_string()));
    }

    #[test]
    fn test_read_root_shell_not_root() {
        let tmp = temp_dir();
        let passwd = tmp.path().join("passwd");
        fs::write(&passwd, "nobody:x:65534:65534:Nobody:/:/usr/bin/nologin\n").unwrap();
        let shell = read_root_shell(passwd.to_str().unwrap());
        assert!(shell.is_none());
    }

    #[test]
    fn test_read_root_shell_missing() {
        let shell = read_root_shell("/nonexistent/passwd");
        assert!(shell.is_none());
    }

    // -----------------------------------------------------------------------
    // Credential loading tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_load_credential_from_dir_basic() {
        let tmp = temp_dir();
        fs::write(tmp.path().join("firstboot.locale"), "en_US.UTF-8\n").unwrap();
        let result = load_credential_from_dir(tmp.path(), "firstboot.locale");
        assert_eq!(result, Some("en_US.UTF-8".to_string()));
    }

    #[test]
    fn test_load_credential_from_dir_missing() {
        let tmp = temp_dir();
        let result = load_credential_from_dir(tmp.path(), "firstboot.nonexistent");
        assert!(result.is_none());
    }

    #[test]
    fn test_load_credential_from_dir_empty() {
        let tmp = temp_dir();
        fs::write(tmp.path().join("firstboot.locale"), "  \n").unwrap();
        let result = load_credential_from_dir(tmp.path(), "firstboot.locale");
        assert!(result.is_none());
    }

    #[test]
    fn test_load_credential_from_dir_nonexistent_dir() {
        let result = load_credential_from_dir(Path::new("/nonexistent/dir"), "firstboot.locale");
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // load_settings_from_credentials tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_load_settings_from_credentials_all() {
        let tmp = temp_dir();
        fs::write(tmp.path().join("firstboot.locale"), "de_DE.UTF-8").unwrap();
        fs::write(tmp.path().join("firstboot.locale-messages"), "en_US.UTF-8").unwrap();
        fs::write(tmp.path().join("firstboot.keymap"), "de").unwrap();
        fs::write(tmp.path().join("firstboot.timezone"), "Europe/Berlin").unwrap();
        fs::write(tmp.path().join("firstboot.hostname"), "myhost").unwrap();
        fs::write(
            tmp.path().join("firstboot.machine-id"),
            "aabbccddaabbccddaabbccddaabbccdd",
        )
        .unwrap();
        fs::write(tmp.path().join("firstboot.root-shell"), "/bin/zsh").unwrap();
        fs::write(
            tmp.path().join("firstboot.kernel-command-line"),
            "quiet splash",
        )
        .unwrap();
        fs::write(
            tmp.path().join("firstboot.root-password-hashed"),
            "$6$salt$hash",
        )
        .unwrap();

        let mut settings = Settings::default();
        load_settings_from_credentials_dir(&mut settings, tmp.path());

        assert_eq!(settings.locale, Some("de_DE.UTF-8".to_string()));
        assert_eq!(settings.locale_messages, Some("en_US.UTF-8".to_string()));
        assert_eq!(settings.keymap, Some("de".to_string()));
        assert_eq!(settings.timezone, Some("Europe/Berlin".to_string()));
        assert_eq!(settings.hostname, Some("myhost".to_string()));
        assert_eq!(
            settings.machine_id,
            Some("aabbccddaabbccddaabbccddaabbccdd".to_string())
        );
        assert_eq!(settings.root_shell, Some("/bin/zsh".to_string()));
        assert_eq!(settings.kernel_cmdline, Some("quiet splash".to_string()));
        assert_eq!(settings.root_password, Some("$6$salt$hash".to_string()));
        assert_eq!(settings.root_password_hashed, Some(true));
    }

    #[test]
    fn test_load_settings_from_credentials_no_overwrite() {
        let tmp = temp_dir();
        fs::write(tmp.path().join("firstboot.locale"), "de_DE.UTF-8").unwrap();

        let mut settings = Settings {
            locale: Some("en_US.UTF-8".to_string()),
            ..Default::default()
        };
        load_settings_from_credentials_dir(&mut settings, tmp.path());

        // Pre-set value should not be overwritten
        assert_eq!(settings.locale, Some("en_US.UTF-8".to_string()));
    }

    // -----------------------------------------------------------------------
    // Timezone validation tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_valid_timezone_utc() {
        assert!(is_valid_timezone("UTC", Path::new("/")));
        assert!(is_valid_timezone("Etc/UTC", Path::new("/")));
    }

    #[test]
    fn test_is_valid_timezone_empty() {
        assert!(!is_valid_timezone("", Path::new("/")));
    }

    #[test]
    fn test_is_valid_timezone_absolute() {
        assert!(!is_valid_timezone("/etc/localtime", Path::new("/")));
    }

    #[test]
    fn test_is_valid_timezone_dotdot() {
        assert!(!is_valid_timezone("../etc/shadow", Path::new("/")));
    }

    // -----------------------------------------------------------------------
    // Salt/password generation tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_hash_password_returns_something() {
        // hash_password should return either a proper hash or "!" if no tool available
        let result = hash_password("test");
        assert!(!result.is_empty());
        // Either a $6$ hash or locked "!"
        assert!(result.starts_with("$") || result == "!");
    }

    // -----------------------------------------------------------------------
    // run() integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_run_locale_and_hostname() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "--locale=en_US.UTF-8".to_string(),
            "--hostname=testhost".to_string(),
            "--force".to_string(),
        ];
        run(&args).unwrap();
        let locale = fs::read_to_string(tmp.path().join("etc/locale.conf")).unwrap();
        assert!(locale.contains("LANG=en_US.UTF-8"));
        let hostname = fs::read_to_string(tmp.path().join("etc/hostname")).unwrap();
        assert_eq!(hostname, "testhost\n");
    }

    #[test]
    fn test_run_reset_hostname() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(tmp.path().join("etc/hostname"), "myhost\n").unwrap();
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "--reset-hostname".to_string(),
        ];
        run(&args).unwrap();
        assert!(!tmp.path().join("etc/hostname").exists());
    }

    #[test]
    fn test_run_nonexistent_root() {
        let args = vec!["--root=/nonexistent/path/xyz".to_string()];
        let result = run(&args);
        assert!(result.is_err());
    }

    #[test]
    /// A machine-id in an explicit --root tree must NOT suppress configuration.
    ///
    /// This asserted the opposite until 2026-07-28. The already-booted check is
    /// about the running system, and applying it to a --root tree meant that
    /// once anything wrote a machine-id there, every later firstboot call
    /// against that tree silently did nothing.
    /// TEST-74-AUX-UTILS.firstboot does exactly that: it sets --machine-id= and
    /// then finds --root-password= ignored. Upstream has no such blanket guard;
    /// first-boot-ness gates the service via ConditionFirstBoot=.
    fn test_run_already_booted_root_still_applies() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(
            tmp.path().join("etc/machine-id"),
            "0123456789abcdef0123456789abcdef\n",
        )
        .unwrap();
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "--locale=en_US.UTF-8".to_string(),
        ];
        run(&args).unwrap();
        let content = fs::read_to_string(tmp.path().join("etc/locale.conf")).unwrap();
        assert!(content.contains("en_US.UTF-8"));
    }

    #[test]
    fn test_run_already_booted_force() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(
            tmp.path().join("etc/machine-id"),
            "0123456789abcdef0123456789abcdef\n",
        )
        .unwrap();
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "--locale=en_US.UTF-8".to_string(),
            "--force".to_string(),
        ];
        run(&args).unwrap();
        assert!(tmp.path().join("etc/locale.conf").exists());
    }

    #[test]
    fn test_run_machine_id() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "--machine-id=aabbccddaabbccddaabbccddaabbccdd".to_string(),
        ];
        run(&args).unwrap();
        let content = fs::read_to_string(tmp.path().join("etc/machine-id")).unwrap();
        assert_eq!(content, "aabbccddaabbccddaabbccddaabbccdd\n");
    }

    #[test]
    fn test_run_timezone() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "--timezone=Europe/Berlin".to_string(),
        ];
        run(&args).unwrap();
        let link = fs::read_link(tmp.path().join("etc/localtime")).unwrap();
        assert_eq!(
            link.to_string_lossy(),
            "../usr/share/zoneinfo/Europe/Berlin"
        );
    }

    #[test]
    fn test_run_kernel_cmdline() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "--kernel-command-line=quiet splash".to_string(),
        ];
        run(&args).unwrap();
        let content = fs::read_to_string(tmp.path().join("etc/kernel/cmdline")).unwrap();
        assert_eq!(content, "quiet splash\n");
    }

    #[test]
    fn test_run_setup_machine_id() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "--setup-machine-id".to_string(),
        ];
        run(&args).unwrap();
        let content = fs::read_to_string(tmp.path().join("etc/machine-id")).unwrap();
        assert!(is_valid_machine_id(content.trim()));
    }

    #[test]
    fn test_run_delete_root_password() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(
            tmp.path().join("etc/shadow"),
            "root:$6$existing$hash:19000:0:99999:7:::\n",
        )
        .unwrap();
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "--delete-root-password".to_string(),
            "--force".to_string(),
        ];
        run(&args).unwrap();
        let content = fs::read_to_string(tmp.path().join("etc/shadow")).unwrap();
        assert!(content.contains("root::"));
    }

    #[test]
    fn test_run_root_shell() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(
            tmp.path().join("etc/passwd"),
            "root:x:0:0:root:/root:/bin/bash\n",
        )
        .unwrap();
        let args = vec![
            format!("--root={}", tmp.path().display()),
            "--root-shell=/bin/zsh".to_string(),
            "--force".to_string(),
        ];
        run(&args).unwrap();
        let content = fs::read_to_string(tmp.path().join("etc/passwd")).unwrap();
        assert!(content.contains("/bin/zsh"));
    }

    #[test]
    fn test_run_multiple_resets() {
        let tmp = temp_dir();
        setup_root(tmp.path());
        fs::write(tmp.path().join("etc/hostname"), "myhost\n").unwrap();
        fs::write(tmp.path().join("etc/locale.conf"), "LANG=C\n").unwrap();
        let link_path = tmp.path().join("etc/localtime");
        symlink("../usr/share/zoneinfo/UTC", &link_path).unwrap();

        let args = vec![
            format!("--root={}", tmp.path().display()),
            "--reset-hostname".to_string(),
            "--reset-locale".to_string(),
            "--reset-timezone".to_string(),
        ];
        run(&args).unwrap();
        assert!(!tmp.path().join("etc/hostname").exists());
        assert!(!tmp.path().join("etc/locale.conf").exists());
        assert!(!tmp.path().join("etc/localtime").exists());
    }

    #[test]
    fn test_run_empty_args_succeeds() {
        // When no settings, no resets, no prompts, the root is /
        // which exists, but system is already booted. Should just skip.
        // Since we pass a tempdir without machine-id it should just say "no settings"
        let tmp = temp_dir();
        setup_root(tmp.path());
        let args = vec![format!("--root={}", tmp.path().display())];
        run(&args).unwrap();
    }

    // -----------------------------------------------------------------------
    // ensure_parent_dir tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_ensure_parent_dir_creates() {
        let tmp = temp_dir();
        let path = tmp.path().join("a/b/c/file.txt");
        ensure_parent_dir(&path).unwrap();
        assert!(tmp.path().join("a/b/c").is_dir());
    }

    #[test]
    fn test_ensure_parent_dir_existing() {
        let tmp = temp_dir();
        fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        let path = tmp.path().join("a/b/file.txt");
        ensure_parent_dir(&path).unwrap();
        assert!(tmp.path().join("a/b").is_dir());
    }

    // -----------------------------------------------------------------------
    // list_available_shells tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_list_available_shells_returns_nonzero() {
        let shells = list_available_shells();
        assert!(!shells.is_empty());
    }

    // -----------------------------------------------------------------------
    // list_available_locales tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_list_available_locales_returns_nonzero() {
        let locales = list_available_locales();
        assert!(!locales.is_empty());
    }
}

use log::trace;

use crate::units::{
    ParsedCommonConfig, ParsedFile, ParsedSection, ParsingErrorReason, parse_install_section,
    parse_unit_section,
};
use std::path::PathBuf;

/// Parsed configuration for a `.mount` unit file.
pub struct ParsedMountConfig {
    pub common: ParsedCommonConfig,
    pub mount: ParsedMountSection,
}

/// Parsed `[Mount]` section from a mount unit file.
///
/// See systemd.mount(5) for full documentation of these options.
pub struct ParsedMountSection {
    /// What= — the absolute path of a device node, file, or other resource to mount.
    /// Required for mount units that actually perform a mount operation.
    pub what: String,

    /// Where= — the absolute path of a directory for the mount point.
    /// If the mount unit is auto-generated from the unit name, this is derived
    /// from the unit name. For explicitly written unit files, this must match
    /// the unit name.
    pub where_: String,

    /// Type= — the file system type (e.g. "ext4", "tmpfs", "nfs", "btrfs").
    /// Corresponds to the -t option of mount(8). If omitted, the kernel
    /// auto-detects the filesystem type.
    pub fs_type: Option<String>,

    /// Options= — mount options as a comma-separated string.
    /// Corresponds to the -o option of mount(8).
    pub options: Option<String>,

    /// SloppyOptions= — if true, relaxed option parsing is enabled and
    /// unknown mount options are tolerated. Defaults to false.
    pub sloppy_options: bool,

    /// LazyUnmount= — if true, a lazy unmount is performed when deactivating
    /// the mount unit (MNT_DETACH). Defaults to false.
    pub lazy_unmount: bool,

    /// ReadWriteOnly= — if true, the mount operation fails if the filesystem
    /// cannot be mounted read-write. Defaults to false.
    pub read_write_only: bool,

    /// ForceUnmount= — if true, a forced unmount is attempted (MNT_FORCE)
    /// when deactivating the mount. Defaults to false.
    pub force_unmount: bool,

    /// DirectoryMode= — the file system access mode used for creating the
    /// mount point directory if it does not exist. Defaults to 0o755.
    pub directory_mode: u32,

    /// TimeoutSec= — timeout for the mount operation. If the mount does not
    /// complete within this time, the mount unit enters a failed state.
    /// Defaults to None (use the manager default, typically 90 seconds).
    pub timeout_sec: Option<u64>,

    /// ConfigurationDirectory= — directories under /etc created on start.
    pub configuration_directory: Vec<String>,
    /// RuntimeDirectory= — directories under /run created on start, removed on stop.
    pub runtime_directory: Vec<String>,
    /// StateDirectory= — directories under /var/lib created on start.
    pub state_directory: Vec<String>,
    /// CacheDirectory= — directories under /var/cache created on start.
    pub cache_directory: Vec<String>,
    /// LogsDirectory= — directories under /var/log created on start.
    pub logs_directory: Vec<String>,
}

impl Default for ParsedMountSection {
    fn default() -> Self {
        Self {
            what: String::new(),
            where_: String::new(),
            fs_type: None,
            options: None,
            sloppy_options: false,
            lazy_unmount: false,
            read_write_only: false,
            force_unmount: false,
            directory_mode: 0o755,
            timeout_sec: None,
            configuration_directory: Vec::new(),
            runtime_directory: Vec::new(),
            state_directory: Vec::new(),
            cache_directory: Vec::new(),
            logs_directory: Vec::new(),
        }
    }
}

/// Parse a mount unit file into a `ParsedMountConfig`.
///
/// Mount unit files have `[Unit]`, `[Mount]`, and optionally `[Install]`
/// sections. The `[Mount]` section contains `What=`, `Where=`, `Type=`,
/// `Options=`, and related settings.
pub fn parse_mount(
    parsed_file: ParsedFile,
    path: &PathBuf,
) -> Result<ParsedMountConfig, ParsingErrorReason> {
    let mut install_config = None;
    let mut unit_config = None;
    let mut mount_section = ParsedMountSection::default();
    let mut has_mount_section = false;

    for (name, section) in parsed_file {
        match name.as_str() {
            "[Unit]" => {
                unit_config = Some(parse_unit_section(section)?);
            }
            "[Install]" => {
                install_config = Some(parse_install_section(section)?);
            }
            "[Mount]" => {
                has_mount_section = true;
                parse_mount_section(&section, &mut mount_section)?;
            }
            _ if name.starts_with("[X-") || name.starts_with("[x-") => {
                trace!("Silently ignoring vendor extension section in mount unit {path:?}: {name}");
            }
            _ => {
                trace!("Ignoring unknown section in mount unit {path:?}: {name}");
            }
        }
    }

    let unit_name = path.file_name().unwrap().to_str().unwrap().to_owned();

    // If Where= was not explicitly set in the [Mount] section, derive it
    // from the unit name. The unit name encoding is the reverse of
    // path_to_mount_unit_name: `-.mount` → `/`, `var-log.mount` → `/var/log`.
    if mount_section.where_.is_empty() {
        mount_section.where_ = mount_unit_name_to_path(&unit_name);
    }

    if !has_mount_section {
        trace!("Mount unit {path:?} has no [Mount] section; using defaults derived from unit name");
    }

    Ok(ParsedMountConfig {
        common: ParsedCommonConfig {
            name: unit_name,
            unit: unit_config.unwrap_or_default(),
            install: install_config.unwrap_or_default(),
            fragment_path: Some(path.clone()),
        },
        mount: mount_section,
    })
}

/// Parse the `[Mount]` section key-value pairs into a `ParsedMountSection`.
///
/// The `ParsedSection` is a `HashMap<String, Vec<(u32, String)>>` where keys
/// are uppercased setting names and values are `(line_number, value)` pairs.
fn parse_mount_section(
    section: &ParsedSection,
    mount: &mut ParsedMountSection,
) -> Result<(), ParsingErrorReason> {
    if let Some(values) = section.get("WHAT")
        && let Some((_line, value)) = values.last()
    {
        mount.what = value.clone();
    }
    if let Some(values) = section.get("WHERE")
        && let Some((_line, value)) = values.last()
    {
        mount.where_ = value.clone();
    }
    if let Some(values) = section.get("TYPE")
        && let Some((_line, value)) = values.last()
    {
        mount.fs_type = if value.is_empty() {
            None
        } else {
            Some(value.clone())
        };
    }
    if let Some(values) = section.get("OPTIONS")
        && let Some((_line, value)) = values.last()
    {
        mount.options = if value.is_empty() {
            None
        } else {
            Some(value.clone())
        };
    }
    if let Some(values) = section.get("SLOPPYOPTIONS")
        && let Some((_line, value)) = values.last()
    {
        mount.sloppy_options = parse_bool_value(value);
    }
    if let Some(values) = section.get("LAZYUNMOUNT")
        && let Some((_line, value)) = values.last()
    {
        mount.lazy_unmount = parse_bool_value(value);
    }
    if let Some(values) = section.get("READWRITEONLY")
        && let Some((_line, value)) = values.last()
    {
        mount.read_write_only = parse_bool_value(value);
    }
    if let Some(values) = section.get("FORCEUNMOUNT")
        && let Some((_line, value)) = values.last()
    {
        mount.force_unmount = parse_bool_value(value);
    }
    if let Some(values) = section.get("DIRECTORYMODE")
        && let Some((_line, value)) = values.last()
    {
        mount.directory_mode = parse_directory_mode(value);
    }
    if let Some(values) = section.get("TIMEOUTSEC")
        && let Some((_line, value)) = values.last()
    {
        mount.timeout_sec = parse_mount_timeout_sec(value);
    }
    // Exec directories (ConfigurationDirectory=, RuntimeDirectory=, etc.):
    // whitespace-separated names, accumulated across all occurrences.
    fn collect_dir(section: &ParsedSection, key: &str) -> Vec<String> {
        section
            .get(key)
            .map(|values| {
                values
                    .iter()
                    .flat_map(|(_line, val)| val.split_whitespace().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }
    mount.configuration_directory = collect_dir(section, "CONFIGURATIONDIRECTORY");
    mount.runtime_directory = collect_dir(section, "RUNTIMEDIRECTORY");
    mount.state_directory = collect_dir(section, "STATEDIRECTORY");
    mount.cache_directory = collect_dir(section, "CACHEDIRECTORY");
    mount.logs_directory = collect_dir(section, "LOGSDIRECTORY");
    // Log any unrecognized keys at trace level
    let known_keys = [
        "WHAT",
        "WHERE",
        "TYPE",
        "OPTIONS",
        "SLOPPYOPTIONS",
        "LAZYUNMOUNT",
        "READWRITEONLY",
        "FORCEUNMOUNT",
        "DIRECTORYMODE",
        "TIMEOUTSEC",
        "CONFIGURATIONDIRECTORY",
        "RUNTIMEDIRECTORY",
        "STATEDIRECTORY",
        "CACHEDIRECTORY",
        "LOGSDIRECTORY",
    ];
    for key in section.keys() {
        if !known_keys.contains(&key.as_str()) {
            trace!("Ignoring unknown [Mount] setting: {key}");
        }
    }
    Ok(())
}

/// Parse a boolean value from a unit file setting.
/// Accepts "yes", "true", "1", "on" as true; everything else is false.
fn parse_bool_value(value: &str) -> bool {
    matches!(
        value.trim().to_lowercase().as_str(),
        "yes" | "true" | "1" | "on"
    )
}

/// Parse a directory mode value (octal). Defaults to 0o755 on parse failure.
fn parse_directory_mode(value: &str) -> u32 {
    let trimmed = value.trim();
    u32::from_str_radix(trimmed, 8).unwrap_or(0o755)
}

/// Parse a timeout value in seconds using the full systemd timespan syntax.
/// Returns None for "infinity" or "0".
fn parse_mount_timeout_sec(value: &str) -> Option<u64> {
    let trimmed = value.trim().to_lowercase();
    if trimmed == "infinity" || trimmed == "0" {
        return None;
    }
    crate::units::from_parsed_config::parse_timespan(value).map(|d| d.as_secs())
}

/// Convert a mount unit name back to the filesystem path it represents.
///
/// This is the reverse of `path_to_mount_unit_name`:
/// - `-.mount` → `/`
/// - `var.mount` → `/var`
/// - `var-log.mount` → `/var/log`
///
/// Note: this does NOT handle systemd's full unit name unescaping (e.g.
/// `\x2d` for literal hyphens). That can be added later if needed.
pub fn mount_unit_name_to_path(unit_name: &str) -> String {
    let without_suffix = unit_name.strip_suffix(".mount").unwrap_or(unit_name);
    if without_suffix == "-" {
        "/".to_owned()
    } else {
        format!("/{}", without_suffix.replace('-', "/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::parse_file;

    #[test]
    fn test_mount_unit_name_to_path() {
        assert_eq!(mount_unit_name_to_path("-.mount"), "/");
        assert_eq!(mount_unit_name_to_path("var.mount"), "/var");
        assert_eq!(mount_unit_name_to_path("var-log.mount"), "/var/log");
        assert_eq!(
            mount_unit_name_to_path("var-log-journal.mount"),
            "/var/log/journal"
        );
        assert_eq!(mount_unit_name_to_path("home.mount"), "/home");
        assert_eq!(
            mount_unit_name_to_path("run-user-1000.mount"),
            "/run/user/1000"
        );
    }

    #[test]
    fn test_parse_mount_basic() {
        let content = r#"
[Unit]
Description=Mount /var/log

[Mount]
What=/dev/sda2
Where=/var/log
Type=ext4
Options=defaults,noatime
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/var-log.mount");
        let config = parse_mount(parsed_file, &path).unwrap();

        assert_eq!(config.mount.what, "/dev/sda2");
        assert_eq!(config.mount.where_, "/var/log");
        assert_eq!(config.mount.fs_type, Some("ext4".to_owned()));
        assert_eq!(config.mount.options, Some("defaults,noatime".to_owned()));
        assert!(!config.mount.sloppy_options);
        assert!(!config.mount.lazy_unmount);
        assert!(!config.mount.read_write_only);
        assert!(!config.mount.force_unmount);
        assert_eq!(config.mount.directory_mode, 0o755);
        assert_eq!(config.mount.timeout_sec, None);
        assert_eq!(config.common.unit.description, "Mount /var/log");
    }

    #[test]
    fn test_parse_mount_tmpfs() {
        let content = r#"
[Mount]
What=tmpfs
Where=/run/wrappers
Type=tmpfs
Options=mode=755,size=50%
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/run-wrappers.mount");
        let config = parse_mount(parsed_file, &path).unwrap();

        assert_eq!(config.mount.what, "tmpfs");
        assert_eq!(config.mount.where_, "/run/wrappers");
        assert_eq!(config.mount.fs_type, Some("tmpfs".to_owned()));
        assert_eq!(config.mount.options, Some("mode=755,size=50%".to_owned()));
    }

    #[test]
    fn test_parse_mount_with_options() {
        let content = r#"
[Mount]
What=/dev/sda1
Where=/boot
Type=vfat
Options=umask=0077
SloppyOptions=yes
LazyUnmount=true
ReadWriteOnly=yes
ForceUnmount=on
DirectoryMode=0700
TimeoutSec=60
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/boot.mount");
        let config = parse_mount(parsed_file, &path).unwrap();

        assert_eq!(config.mount.what, "/dev/sda1");
        assert_eq!(config.mount.where_, "/boot");
        assert_eq!(config.mount.fs_type, Some("vfat".to_owned()));
        assert!(config.mount.sloppy_options);
        assert!(config.mount.lazy_unmount);
        assert!(config.mount.read_write_only);
        assert!(config.mount.force_unmount);
        assert_eq!(config.mount.directory_mode, 0o700);
        assert_eq!(config.mount.timeout_sec, Some(60));
    }

    #[test]
    fn test_parse_mount_where_from_name() {
        // When Where= is not specified, it should be derived from the unit name
        let content = r#"
[Mount]
What=/dev/sda2
Type=ext4
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/var-log.mount");
        let config = parse_mount(parsed_file, &path).unwrap();

        assert_eq!(config.mount.where_, "/var/log");
    }

    #[test]
    fn test_parse_mount_root() {
        let content = r#"
[Mount]
What=/dev/sda1
Type=ext4
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/-.mount");
        let config = parse_mount(parsed_file, &path).unwrap();

        assert_eq!(config.mount.where_, "/");
    }

    #[test]
    fn test_parse_mount_no_mount_section() {
        // A mount unit without a [Mount] section should still parse
        let content = r#"
[Unit]
Description=Test mount point
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/tmp.mount");
        let config = parse_mount(parsed_file, &path).unwrap();

        assert_eq!(config.mount.where_, "/tmp");
        assert!(config.mount.what.is_empty());
    }

    #[test]
    fn test_parse_mount_with_install() {
        let content = r#"
[Unit]
Description=Mount /home

[Mount]
What=/dev/sda3
Where=/home
Type=ext4

[Install]
WantedBy=local-fs.target
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/home.mount");
        let config = parse_mount(parsed_file, &path).unwrap();

        assert_eq!(config.mount.what, "/dev/sda3");
        assert_eq!(config.mount.where_, "/home");
        assert!(
            config
                .common
                .install
                .wanted_by
                .contains(&"local-fs.target".to_owned())
        );
    }

    #[test]
    fn test_parse_bool_value() {
        assert!(parse_bool_value("yes"));
        assert!(parse_bool_value("true"));
        assert!(parse_bool_value("1"));
        assert!(parse_bool_value("on"));
        assert!(parse_bool_value("Yes"));
        assert!(parse_bool_value("TRUE"));
        assert!(parse_bool_value("ON"));
        assert!(!parse_bool_value("no"));
        assert!(!parse_bool_value("false"));
        assert!(!parse_bool_value("0"));
        assert!(!parse_bool_value("off"));
        assert!(!parse_bool_value(""));
    }

    #[test]
    fn test_parse_directory_mode() {
        assert_eq!(parse_directory_mode("0755"), 0o755);
        assert_eq!(parse_directory_mode("0700"), 0o700);
        assert_eq!(parse_directory_mode("0555"), 0o555);
        assert_eq!(parse_directory_mode("0777"), 0o777);
        // Invalid falls back to 0o755
        assert_eq!(parse_directory_mode("invalid"), 0o755);
    }

    #[test]
    fn test_parse_mount_timeout_sec() {
        assert_eq!(parse_mount_timeout_sec("60"), Some(60));
        assert_eq!(parse_mount_timeout_sec("90"), Some(90));
        assert_eq!(parse_mount_timeout_sec("0"), None);
        assert_eq!(parse_mount_timeout_sec("infinity"), None);
        assert_eq!(parse_mount_timeout_sec("Infinity"), None);
        // Full timespan syntax now supported
        assert_eq!(parse_mount_timeout_sec("1min"), Some(60));
        assert_eq!(parse_mount_timeout_sec("1min 30s"), Some(90));
        assert_eq!(parse_mount_timeout_sec("2h"), Some(7200));
        assert_eq!(parse_mount_timeout_sec("2hrs"), Some(7200));
        assert_eq!(parse_mount_timeout_sec("500ms"), Some(0)); // sub-second rounds to 0
    }

    #[test]
    fn test_parse_mount_vendor_extension() {
        // Vendor extension sections should be silently ignored
        let content = r#"
[Unit]
Description=Test mount

[Mount]
What=tmpfs
Where=/tmp
Type=tmpfs

[X-Custom]
SomeKey=SomeValue
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/tmp.mount");
        let config = parse_mount(parsed_file, &path).unwrap();

        assert_eq!(config.mount.what, "tmpfs");
        assert_eq!(config.mount.where_, "/tmp");
    }
}

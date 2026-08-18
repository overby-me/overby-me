use log::trace;

use crate::units::{
    ParsedCommonConfig, ParsedFile, ParsedSection, ParsedSwapConfig, ParsingErrorReason,
    parse_install_section, parse_unit_section,
};
use std::path::PathBuf;

/// Parsed `[Swap]` section from a swap unit file.
///
/// See systemd.swap(5) for full documentation of these options.
#[derive(Default)]
pub struct ParsedSwapSection {
    /// What= — the absolute path of a device node or file to use as swap space.
    /// For swap units auto-generated from the unit name, this is derived from
    /// the unit name. For explicitly written unit files, this should be set.
    pub what: String,

    /// Priority= — the swap priority (an integer passed to swapon(2) via
    /// SWAP_FLAG_PREFER). If not set, the kernel default is used.
    /// Range: 0–32767 (matching swapon(2) semantics).
    pub priority: Option<i32>,

    /// Options= — options string for swapon(8), comma-separated.
    /// Passed as-is. Common options: `discard`, `pri=N`, `nofail`.
    pub options: Option<String>,

    /// TimeoutSec= — timeout for the swapon/swapoff operation. If the
    /// operation does not complete within this time, the swap unit enters
    /// a failed state. Defaults to None (use the manager default, typically
    /// 90 seconds).
    pub timeout_sec: Option<u64>,
}

/// Parse a swap unit file into a `ParsedSwapConfig`.
///
/// Swap unit files have `[Unit]`, `[Swap]`, and optionally `[Install]`
/// sections. The `[Swap]` section contains `What=`, `Priority=`, `Options=`,
/// and `TimeoutSec=`.
pub fn parse_swap(
    parsed_file: ParsedFile,
    path: &PathBuf,
) -> Result<ParsedSwapConfig, ParsingErrorReason> {
    let mut install_config = None;
    let mut unit_config = None;
    let mut swap_section = ParsedSwapSection::default();
    let mut has_swap_section = false;

    for (name, section) in parsed_file {
        match name.as_str() {
            "[Unit]" => {
                unit_config = Some(parse_unit_section(section)?);
            }
            "[Install]" => {
                install_config = Some(parse_install_section(section)?);
            }
            "[Swap]" => {
                has_swap_section = true;
                parse_swap_section(&section, &mut swap_section)?;
            }
            _ if name.starts_with("[X-") || name.starts_with("[x-") => {
                trace!("Silently ignoring vendor extension section in swap unit {path:?}: {name}");
            }
            _ => {
                trace!("Ignoring unknown section in swap unit {path:?}: {name}");
            }
        }
    }

    let unit_name = path.file_name().unwrap().to_str().unwrap().to_owned();

    // If What= was not explicitly set in the [Swap] section, derive it
    // from the unit name. The unit name encoding follows the same pattern
    // as mount units: `dev-sda2.swap` → `/dev/sda2`.
    if swap_section.what.is_empty() {
        swap_section.what = swap_unit_name_to_path(&unit_name);
    }

    if !has_swap_section {
        trace!("Swap unit {path:?} has no [Swap] section; using defaults derived from unit name");
    }

    Ok(ParsedSwapConfig {
        common: ParsedCommonConfig {
            name: unit_name,
            unit: unit_config.unwrap_or_default(),
            install: install_config.unwrap_or_default(),
            fragment_path: Some(path.clone()),
        },
        swap: swap_section,
    })
}

/// Parse the `[Swap]` section key-value pairs into a `ParsedSwapSection`.
fn parse_swap_section(
    section: &ParsedSection,
    swap: &mut ParsedSwapSection,
) -> Result<(), ParsingErrorReason> {
    if let Some(values) = section.get("WHAT")
        && let Some((_line, value)) = values.last()
    {
        swap.what = value.clone();
    }
    if let Some(values) = section.get("PRIORITY")
        && let Some((_line, value)) = values.last()
    {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            swap.priority = None;
        } else if let Ok(p) = trimmed.parse::<i32>() {
            swap.priority = Some(p);
        } else {
            trace!("Invalid Priority= value in [Swap] section: {trimmed}");
        }
    }
    if let Some(values) = section.get("OPTIONS")
        && let Some((_line, value)) = values.last()
    {
        swap.options = if value.is_empty() {
            None
        } else {
            Some(value.clone())
        };
    }
    if let Some(values) = section.get("TIMEOUTSEC")
        && let Some((_line, value)) = values.last()
    {
        swap.timeout_sec = parse_timeout_sec(value);
    }

    // Log any unrecognized keys at trace level
    let known_keys = ["WHAT", "PRIORITY", "OPTIONS", "TIMEOUTSEC"];
    for key in section.keys() {
        if !known_keys.contains(&key.as_str()) {
            trace!("Ignoring unknown [Swap] setting: {key}");
        }
    }
    Ok(())
}

/// Parse a timeout value in seconds using the full systemd timespan syntax.
/// Returns None for "infinity" or "0".
fn parse_timeout_sec(value: &str) -> Option<u64> {
    let trimmed = value.trim().to_lowercase();
    if trimmed == "infinity" || trimmed == "0" {
        return None;
    }
    crate::units::from_parsed_config::parse_timespan(value).map(|d| d.as_secs())
}

/// Convert a swap unit name back to the device/file path it represents.
///
/// This follows the same encoding as mount units:
/// - `dev-sda2.swap` → `/dev/sda2`
/// - `swapfile.swap` → `/swapfile`
/// - `dev-disk-by\\x2did-foo.swap` → `/dev/disk/by-id/foo` (with unescaping)
///
/// Note: full systemd unit name unescaping (e.g. `\x2d` for literal hyphens)
/// is handled here for the common case. Complex escaping can be extended later.
pub fn swap_unit_name_to_path(unit_name: &str) -> String {
    let without_suffix = unit_name.strip_suffix(".swap").unwrap_or(unit_name);
    if without_suffix == "-" {
        "/".to_owned()
    } else {
        // First, protect escaped hyphens (\x2d) with a placeholder so the
        // blanket '-' → '/' replacement doesn't clobber them.
        let protected = without_suffix.replace("\\x2d", "\x00HYPHEN\x00");
        // Replace path-separator dashes with slashes
        let slashed = protected.replace('-', "/");
        // Restore escaped hyphens and handle \x2f (escaped slash)
        let unescaped = slashed.replace("\x00HYPHEN\x00", "-").replace("\\x2f", "/");
        format!("/{}", unescaped)
    }
}

/// Convert a device/file path to a swap unit name.
///
/// `/dev/sda2` → `dev-sda2.swap`
/// `/swapfile` → `swapfile.swap`
pub fn path_to_swap_unit_name(path: &str) -> String {
    let stripped = path.strip_prefix('/').unwrap_or(path);
    if stripped.is_empty() {
        "-.swap".to_owned()
    } else {
        format!("{}.swap", stripped.replace('/', "-"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::parse_file;

    #[test]
    fn test_swap_unit_name_to_path() {
        assert_eq!(swap_unit_name_to_path("dev-sda2.swap"), "/dev/sda2");
        assert_eq!(
            swap_unit_name_to_path("dev-disk-by\\x2did-foo.swap"),
            "/dev/disk/by-id/foo"
        );
        assert_eq!(swap_unit_name_to_path("swapfile.swap"), "/swapfile");
        assert_eq!(swap_unit_name_to_path("-.swap"), "/");
    }

    #[test]
    fn test_path_to_swap_unit_name() {
        assert_eq!(path_to_swap_unit_name("/dev/sda2"), "dev-sda2.swap");
        assert_eq!(path_to_swap_unit_name("/swapfile"), "swapfile.swap");
        assert_eq!(path_to_swap_unit_name("/var/lib/swap"), "var-lib-swap.swap");
        assert_eq!(path_to_swap_unit_name("/"), "-.swap");
    }

    #[test]
    fn test_parse_swap_basic() {
        let content = r#"
[Unit]
Description=Swap on /dev/sda2

[Swap]
What=/dev/sda2
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/dev-sda2.swap");
        let config = parse_swap(parsed_file, &path).unwrap();

        assert_eq!(config.swap.what, "/dev/sda2");
        assert_eq!(config.swap.priority, None);
        assert_eq!(config.swap.options, None);
        assert_eq!(config.swap.timeout_sec, None);
        assert_eq!(config.common.unit.description, "Swap on /dev/sda2");
    }

    #[test]
    fn test_parse_swap_with_priority() {
        let content = r#"
[Swap]
What=/dev/sda2
Priority=10
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/dev-sda2.swap");
        let config = parse_swap(parsed_file, &path).unwrap();

        assert_eq!(config.swap.what, "/dev/sda2");
        assert_eq!(config.swap.priority, Some(10));
    }

    #[test]
    fn test_parse_swap_with_options() {
        let content = r#"
[Swap]
What=/dev/sda2
Options=discard
Priority=5
TimeoutSec=60
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/dev-sda2.swap");
        let config = parse_swap(parsed_file, &path).unwrap();

        assert_eq!(config.swap.what, "/dev/sda2");
        assert_eq!(config.swap.options, Some("discard".to_owned()));
        assert_eq!(config.swap.priority, Some(5));
        assert_eq!(config.swap.timeout_sec, Some(60));
    }

    #[test]
    fn test_parse_swap_what_from_name() {
        // When What= is not specified, derive from unit name
        let content = r#"
[Swap]
Priority=1
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/dev-sda2.swap");
        let config = parse_swap(parsed_file, &path).unwrap();

        assert_eq!(config.swap.what, "/dev/sda2");
    }

    #[test]
    fn test_parse_swap_no_swap_section() {
        // A swap unit without a [Swap] section should still parse
        let content = r#"
[Unit]
Description=Test swap
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/dev-sda2.swap");
        let config = parse_swap(parsed_file, &path).unwrap();

        assert_eq!(config.swap.what, "/dev/sda2");
        assert_eq!(config.swap.priority, None);
    }

    #[test]
    fn test_parse_swap_with_install() {
        let content = r#"
[Unit]
Description=Swap /dev/sda2

[Swap]
What=/dev/sda2

[Install]
WantedBy=swap.target
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/dev-sda2.swap");
        let config = parse_swap(parsed_file, &path).unwrap();

        assert_eq!(config.swap.what, "/dev/sda2");
        assert!(
            config
                .common
                .install
                .wanted_by
                .contains(&"swap.target".to_owned())
        );
    }

    #[test]
    fn test_parse_swap_negative_priority() {
        let content = r#"
[Swap]
What=/dev/sda2
Priority=-1
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/dev-sda2.swap");
        let config = parse_swap(parsed_file, &path).unwrap();

        assert_eq!(config.swap.priority, Some(-1));
    }

    #[test]
    fn test_parse_swap_empty_options_reset() {
        let content = r#"
[Swap]
What=/dev/sda2
Options=discard
Options=
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/dev-sda2.swap");
        let config = parse_swap(parsed_file, &path).unwrap();

        // Empty Options= should reset to None
        assert_eq!(config.swap.options, None);
    }

    #[test]
    fn test_parse_swap_timeout_infinity() {
        let content = r#"
[Swap]
What=/dev/sda2
TimeoutSec=infinity
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/dev-sda2.swap");
        let config = parse_swap(parsed_file, &path).unwrap();

        assert_eq!(config.swap.timeout_sec, None);
    }

    #[test]
    fn test_parse_swap_vendor_extension() {
        let content = r#"
[Unit]
Description=Test swap

[Swap]
What=/dev/sda2

[X-Custom]
SomeKey=SomeValue
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/dev-sda2.swap");
        let config = parse_swap(parsed_file, &path).unwrap();

        assert_eq!(config.swap.what, "/dev/sda2");
    }

    #[test]
    fn test_parse_swap_unknown_keys_ignored() {
        let content = r#"
[Swap]
What=/dev/sda2
SomeUnknownKey=value
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/dev-sda2.swap");
        let config = parse_swap(parsed_file, &path).unwrap();

        assert_eq!(config.swap.what, "/dev/sda2");
    }

    #[test]
    fn test_parse_swap_priority_invalid_ignored() {
        let content = r#"
[Swap]
What=/dev/sda2
Priority=notanumber
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/dev-sda2.swap");
        let config = parse_swap(parsed_file, &path).unwrap();

        // Invalid priority is silently ignored
        assert_eq!(config.swap.priority, None);
    }

    #[test]
    fn test_parse_swap_priority_empty_resets() {
        let content = r#"
[Swap]
What=/dev/sda2
Priority=10
Priority=
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/dev-sda2.swap");
        let config = parse_swap(parsed_file, &path).unwrap();

        assert_eq!(config.swap.priority, None);
    }

    #[test]
    fn test_parse_swap_swapfile() {
        let content = r#"
[Unit]
Description=Swap file

[Swap]
What=/var/lib/swapfile
Priority=0
Options=discard
"#;
        let parsed_file = parse_file(content).unwrap();
        let path = PathBuf::from("/etc/systemd/system/var-lib-swapfile.swap");
        let config = parse_swap(parsed_file, &path).unwrap();

        assert_eq!(config.swap.what, "/var/lib/swapfile");
        assert_eq!(config.swap.priority, Some(0));
        assert_eq!(config.swap.options, Some("discard".to_owned()));
    }

    #[test]
    fn test_parse_timeout_sec() {
        assert_eq!(parse_timeout_sec("60"), Some(60));
        assert_eq!(parse_timeout_sec("90"), Some(90));
        assert_eq!(parse_timeout_sec("0"), None);
        assert_eq!(parse_timeout_sec("infinity"), None);
        assert_eq!(parse_timeout_sec("Infinity"), None);
    }
}

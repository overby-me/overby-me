use log::trace;

use crate::units::{
    ParsedCommonConfig, ParsedFile, ParsedPathConfig, ParsedPathSection, ParsingErrorReason,
    parse_install_section, parse_unit_section,
};
use std::path::PathBuf;

pub fn parse_path(
    parsed_file: ParsedFile,
    path: &PathBuf,
) -> Result<ParsedPathConfig, ParsingErrorReason> {
    let mut install_config = None;
    let mut unit_config = None;
    let mut path_section = ParsedPathSection::default();

    for (name, section) in parsed_file {
        match name.as_str() {
            "[Unit]" => {
                unit_config = Some(parse_unit_section(section)?);
            }
            "[Install]" => {
                install_config = Some(parse_install_section(section)?);
            }
            "[Path]" => {
                parse_path_section(&section, &mut path_section);
            }
            _ if name.starts_with("[X-") || name.starts_with("[x-") => {
                trace!("Silently ignoring vendor extension section in path unit {path:?}: {name}");
            }
            _ => {
                trace!("Ignoring unknown section in path unit {path:?}: {name}");
            }
        }
    }

    Ok(ParsedPathConfig {
        common: ParsedCommonConfig {
            name: path.file_name().unwrap().to_str().unwrap().to_owned(),
            unit: unit_config.unwrap_or_default(),
            install: install_config.unwrap_or_default(),
            fragment_path: Some(path.clone()),
        },
        path: path_section,
    })
}

fn parse_path_section(
    section: &std::collections::HashMap<String, Vec<(u32, String)>>,
    path_sec: &mut ParsedPathSection,
) {
    // DirectoryMode defaults to 0o755 per systemd.path(5)
    path_sec.directory_mode = 0o755;

    // Collect all key-value pairs sorted by line number so that
    // later assignments override earlier ones (matching systemd semantics).
    let mut entries: Vec<(u32, &str, &str)> = Vec::new();
    for (key, values) in section {
        for (line, value) in values {
            entries.push((*line, key.as_str(), value.as_str()));
        }
    }
    entries.sort_by_key(|(line, _, _)| *line);

    for (_line, key, value) in entries {
        match key {
            "PATHEXISTS" | "PathExists" => {
                if value.is_empty() {
                    // Empty value resets the list (systemd semantics)
                    path_sec.path_exists.retain(|(k, _)| k != "PathExists");
                } else {
                    path_sec
                        .path_exists
                        .push(("PathExists".to_owned(), value.to_owned()));
                }
            }
            "PATHEXISTSGLOB" | "PathExistsGlob" => {
                if value.is_empty() {
                    path_sec.path_exists.retain(|(k, _)| k != "PathExistsGlob");
                } else {
                    path_sec
                        .path_exists
                        .push(("PathExistsGlob".to_owned(), value.to_owned()));
                }
            }
            "PATHCHANGED" | "PathChanged" => {
                if value.is_empty() {
                    path_sec.path_exists.retain(|(k, _)| k != "PathChanged");
                } else {
                    path_sec
                        .path_exists
                        .push(("PathChanged".to_owned(), value.to_owned()));
                }
            }
            "PATHMODIFIED" | "PathModified" => {
                if value.is_empty() {
                    path_sec.path_exists.retain(|(k, _)| k != "PathModified");
                } else {
                    path_sec
                        .path_exists
                        .push(("PathModified".to_owned(), value.to_owned()));
                }
            }
            "DIRECTORYNOTEMPTY" | "DirectoryNotEmpty" => {
                if value.is_empty() {
                    path_sec
                        .path_exists
                        .retain(|(k, _)| k != "DirectoryNotEmpty");
                } else {
                    path_sec
                        .path_exists
                        .push(("DirectoryNotEmpty".to_owned(), value.to_owned()));
                }
            }
            "MAKEDIRECTORY" | "MakeDirectory" => {
                path_sec.make_directory = parse_bool(value);
            }
            "DIRECTORYMODE" | "DirectoryMode" => {
                // Parse octal mode string like "0755" or "755"
                let trimmed = value.trim();
                if let Ok(mode) = u32::from_str_radix(trimmed.trim_start_matches('0'), 8) {
                    path_sec.directory_mode = mode;
                } else if let Ok(mode) = u32::from_str_radix(trimmed, 8) {
                    path_sec.directory_mode = mode;
                } else {
                    trace!("Invalid DirectoryMode value: {value}");
                }
            }
            "TRIGGERLIMITINTERVALSEC" | "TriggerLimitIntervalSec" => {
                path_sec.trigger_limit_interval_sec = if value.is_empty() {
                    None
                } else {
                    Some(value.to_owned())
                };
            }
            "TRIGGERLIMITBURST" | "TriggerLimitBurst" => {
                if let Ok(burst) = value.parse::<u32>() {
                    path_sec.trigger_limit_burst = Some(burst);
                } else if value.is_empty() {
                    path_sec.trigger_limit_burst = None;
                } else {
                    trace!("Invalid TriggerLimitBurst value: {value}");
                }
            }
            "UNIT" | "Unit" => {
                path_sec.unit = if value.is_empty() {
                    None
                } else {
                    Some(value.to_owned())
                };
            }
            other => {
                trace!("Ignoring unknown key in [Path] section: {other}={value}");
            }
        }
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(value.to_lowercase().as_str(), "true" | "yes" | "on" | "1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::parse_file;

    #[test]
    fn test_parse_path_unit_basic() {
        let content = "\
[Unit]
Description=Test path unit

[Path]
PathExists=/tmp/trigger
Unit=my-service.service

[Install]
WantedBy=multi-user.target
";
        let parsed_file = parse_file(content).unwrap();
        let result = parse_path(parsed_file, &PathBuf::from("/test/my.path")).unwrap();
        assert_eq!(result.common.name, "my.path");
        assert_eq!(result.path.path_exists.len(), 1);
        assert_eq!(result.path.path_exists[0].0, "PathExists");
        assert_eq!(result.path.path_exists[0].1, "/tmp/trigger");
        assert_eq!(result.path.unit, Some("my-service.service".to_owned()));
        assert!(!result.path.make_directory);
        assert_eq!(result.path.directory_mode, 0o755);
    }

    #[test]
    fn test_parse_path_unit_multiple_conditions() {
        let content = "\
[Path]
PathExists=/tmp/a
PathChanged=/tmp/b
PathModified=/tmp/c
DirectoryNotEmpty=/tmp/d
PathExistsGlob=/tmp/*.conf
";
        let parsed_file = parse_file(content).unwrap();
        let result = parse_path(parsed_file, &PathBuf::from("/test/multi.path")).unwrap();
        assert_eq!(result.path.path_exists.len(), 5);
        assert_eq!(
            result.path.path_exists[0],
            ("PathExists".to_owned(), "/tmp/a".to_owned())
        );
        assert_eq!(
            result.path.path_exists[1],
            ("PathChanged".to_owned(), "/tmp/b".to_owned())
        );
        assert_eq!(
            result.path.path_exists[2],
            ("PathModified".to_owned(), "/tmp/c".to_owned())
        );
        assert_eq!(
            result.path.path_exists[3],
            ("DirectoryNotEmpty".to_owned(), "/tmp/d".to_owned())
        );
        assert_eq!(
            result.path.path_exists[4],
            ("PathExistsGlob".to_owned(), "/tmp/*.conf".to_owned())
        );
    }

    #[test]
    fn test_parse_path_unit_make_directory() {
        let content = "\
[Path]
PathExists=/var/lib/myapp/trigger
MakeDirectory=yes
DirectoryMode=0700
";
        let parsed_file = parse_file(content).unwrap();
        let result = parse_path(parsed_file, &PathBuf::from("/test/dir.path")).unwrap();
        assert!(result.path.make_directory);
        assert_eq!(result.path.directory_mode, 0o700);
    }

    #[test]
    fn test_parse_path_unit_trigger_limits() {
        let content = "\
[Path]
PathExists=/tmp/trigger
TriggerLimitIntervalSec=5s
TriggerLimitBurst=10
";
        let parsed_file = parse_file(content).unwrap();
        let result = parse_path(parsed_file, &PathBuf::from("/test/limit.path")).unwrap();
        assert_eq!(
            result.path.trigger_limit_interval_sec,
            Some("5s".to_owned())
        );
        assert_eq!(result.path.trigger_limit_burst, Some(10));
    }

    #[test]
    fn test_parse_path_unit_default_unit() {
        let content = "\
[Path]
PathExists=/tmp/trigger
";
        let parsed_file = parse_file(content).unwrap();
        let result = parse_path(parsed_file, &PathBuf::from("/test/myapp.path")).unwrap();
        // When Unit= is not set, it defaults to None (from_parsed_config resolves it to same-name .service)
        assert_eq!(result.path.unit, None);
    }

    #[test]
    fn test_parse_path_unit_empty_resets() {
        let content = "\
[Path]
PathExists=/tmp/a
PathExists=
PathChanged=/tmp/b
";
        let parsed_file = parse_file(content).unwrap();
        let result = parse_path(parsed_file, &PathBuf::from("/test/reset.path")).unwrap();
        // The empty PathExists= should have cleared all PathExists entries,
        // leaving only the PathChanged entry.
        assert_eq!(result.path.path_exists.len(), 1);
        assert_eq!(result.path.path_exists[0].0, "PathChanged");
    }

    #[test]
    fn test_parse_path_unit_directory_mode_formats() {
        // Test various octal format strings
        let content = "\
[Path]
PathExists=/tmp/trigger
DirectoryMode=755
";
        let parsed_file = parse_file(content).unwrap();
        let result = parse_path(parsed_file, &PathBuf::from("/test/mode.path")).unwrap();
        assert_eq!(result.path.directory_mode, 0o755);
    }

    #[test]
    fn test_parse_path_unit_vendor_extension_ignored() {
        let content = "\
[Unit]
Description=Test

[Path]
PathExists=/tmp/trigger

[X-Custom]
Foo=bar
";
        let parsed_file = parse_file(content).unwrap();
        let result = parse_path(parsed_file, &PathBuf::from("/test/vendor.path")).unwrap();
        assert_eq!(result.path.path_exists.len(), 1);
    }

    #[test]
    fn test_parse_path_unit_unknown_keys_ignored() {
        let content = "\
[Path]
PathExists=/tmp/trigger
SomeUnknownKey=value
AnotherKey=123
";
        let parsed_file = parse_file(content).unwrap();
        let result = parse_path(parsed_file, &PathBuf::from("/test/unknown.path")).unwrap();
        assert_eq!(result.path.path_exists.len(), 1);
    }

    #[test]
    fn test_parse_path_unit_make_directory_variants() {
        for (val, expected) in [
            ("yes", true),
            ("true", true),
            ("on", true),
            ("1", true),
            ("no", false),
            ("false", false),
            ("off", false),
            ("0", false),
        ] {
            let content = format!(
                "\
[Path]
PathExists=/tmp/trigger
MakeDirectory={val}
"
            );
            let parsed_file = parse_file(&content).unwrap();
            let result = parse_path(parsed_file, &PathBuf::from("/test/bool.path")).unwrap();
            assert_eq!(
                result.path.make_directory, expected,
                "MakeDirectory={val} should be {expected}"
            );
        }
    }
}

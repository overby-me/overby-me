use log::trace;

use crate::units::{
    ParsedCommonConfig, ParsedFile, ParsedTimerConfig, ParsedTimerSection, ParsingErrorReason,
    parse_install_section, parse_unit_section,
};
use std::path::PathBuf;

pub fn parse_timer(
    parsed_file: ParsedFile,
    path: &PathBuf,
) -> Result<ParsedTimerConfig, ParsingErrorReason> {
    let mut install_config = None;
    let mut unit_config = None;
    let mut timer_section = ParsedTimerSection::default();

    for (name, section) in parsed_file {
        match name.as_str() {
            "[Unit]" => {
                unit_config = Some(parse_unit_section(section)?);
            }
            "[Install]" => {
                install_config = Some(parse_install_section(section)?);
            }
            "[Timer]" => {
                parse_timer_section(&section, &mut timer_section);
            }
            _ if name.starts_with("[X-") || name.starts_with("[x-") => {
                trace!("Silently ignoring vendor extension section in timer unit {path:?}: {name}");
            }
            _ => {
                trace!("Ignoring unknown section in timer unit {path:?}: {name}");
            }
        }
    }

    Ok(ParsedTimerConfig {
        common: ParsedCommonConfig {
            name: path.file_name().unwrap().to_str().unwrap().to_owned(),
            unit: unit_config.unwrap_or_else(Default::default),
            install: install_config.unwrap_or_else(Default::default),
            fragment_path: Some(path.clone()),
        },
        timer: timer_section,
    })
}

fn parse_timer_section(
    section: &std::collections::HashMap<String, Vec<(u32, String)>>,
    timer: &mut ParsedTimerSection,
) {
    // RemainAfterElapse defaults to true per systemd.timer(5)
    timer.remain_after_elapse = true;

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
        // Note: the unit file parser (`parse_section`) converts all keys to
        // uppercase, so we match against uppercase names here.
        match key {
            "ONACTIVESEC" => {
                if value.is_empty() {
                    timer.on_active_sec.clear();
                } else {
                    timer.on_active_sec.push(value.to_owned());
                }
            }
            "ONBOOTSEC" => {
                if value.is_empty() {
                    timer.on_boot_sec.clear();
                } else {
                    timer.on_boot_sec.push(value.to_owned());
                }
            }
            "ONSTARTUPSEC" => {
                if value.is_empty() {
                    timer.on_startup_sec.clear();
                } else {
                    timer.on_startup_sec.push(value.to_owned());
                }
            }
            "ONUNITACTIVESEC" => {
                if value.is_empty() {
                    timer.on_unit_active_sec.clear();
                } else {
                    timer.on_unit_active_sec.push(value.to_owned());
                }
            }
            "ONUNITINACTIVESEC" => {
                if value.is_empty() {
                    timer.on_unit_inactive_sec.clear();
                } else {
                    timer.on_unit_inactive_sec.push(value.to_owned());
                }
            }
            "ONCALENDAR" => {
                if value.is_empty() {
                    timer.on_calendar.clear();
                } else {
                    timer.on_calendar.push(value.to_owned());
                }
            }
            "ACCURACYSEC" => {
                timer.accuracy_sec = if value.is_empty() {
                    None
                } else {
                    Some(value.to_owned())
                };
            }
            "RANDOMIZEDDELAYSEC" => {
                timer.randomized_delay_sec = if value.is_empty() {
                    None
                } else {
                    Some(value.to_owned())
                };
            }
            "FIXEDRANDOMDELAY" => {
                timer.fixed_random_delay = parse_bool(value);
            }
            "PERSISTENT" => {
                timer.persistent = parse_bool(value);
            }
            "WAKESYSTEM" => {
                timer.wake_system = parse_bool(value);
            }
            "REMAINAFTERELAPSE" => {
                timer.remain_after_elapse = parse_bool(value);
            }
            "DEFERREACTIVATION" => {
                timer.defer_reactivation = parse_bool(value);
            }
            "ONCLOCKCHANGE" => {
                timer.on_clock_change = parse_bool(value);
            }
            "ONTIMEZONECHANGE" => {
                timer.on_timezone_change = parse_bool(value);
            }
            "UNIT" => {
                timer.unit = if value.is_empty() {
                    None
                } else {
                    Some(value.to_owned())
                };
            }
            other => {
                trace!("Ignoring unknown key in [Timer] section: {other}={value}");
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
    use crate::units::unit_parsing::unit_parser::parse_file;

    fn parse_timer_from_str(content: &str) -> Result<ParsedTimerConfig, ParsingErrorReason> {
        let path = PathBuf::from("/test/test.timer");
        let parsed_file = parse_file(content)?;
        parse_timer(parsed_file, &path)
    }

    // --- OnClockChange= ---

    #[test]
    fn test_on_clock_change_default() {
        let content = "[Timer]\nOnBootSec=5min\n";
        let config = parse_timer_from_str(content).unwrap();
        assert!(!config.timer.on_clock_change);
    }

    #[test]
    fn test_on_clock_change_true() {
        let content = "[Timer]\nOnClockChange=yes\n";
        let config = parse_timer_from_str(content).unwrap();
        assert!(config.timer.on_clock_change);
    }

    #[test]
    fn test_on_clock_change_false() {
        let content = "[Timer]\nOnClockChange=no\n";
        let config = parse_timer_from_str(content).unwrap();
        assert!(!config.timer.on_clock_change);
    }

    #[test]
    fn test_on_clock_change_true_variant() {
        let content = "[Timer]\nOnClockChange=true\n";
        let config = parse_timer_from_str(content).unwrap();
        assert!(config.timer.on_clock_change);
    }

    #[test]
    fn test_on_clock_change_one() {
        let content = "[Timer]\nOnClockChange=1\n";
        let config = parse_timer_from_str(content).unwrap();
        assert!(config.timer.on_clock_change);
    }

    // --- OnTimezoneChange= ---

    #[test]
    fn test_on_timezone_change_default() {
        let content = "[Timer]\nOnBootSec=5min\n";
        let config = parse_timer_from_str(content).unwrap();
        assert!(!config.timer.on_timezone_change);
    }

    #[test]
    fn test_on_timezone_change_true() {
        let content = "[Timer]\nOnTimezoneChange=yes\n";
        let config = parse_timer_from_str(content).unwrap();
        assert!(config.timer.on_timezone_change);
    }

    #[test]
    fn test_on_timezone_change_false() {
        let content = "[Timer]\nOnTimezoneChange=no\n";
        let config = parse_timer_from_str(content).unwrap();
        assert!(!config.timer.on_timezone_change);
    }

    #[test]
    fn test_on_timezone_change_on() {
        let content = "[Timer]\nOnTimezoneChange=on\n";
        let config = parse_timer_from_str(content).unwrap();
        assert!(config.timer.on_timezone_change);
    }

    // --- Combined ---

    #[test]
    fn test_both_clock_and_timezone_change() {
        let content = "[Timer]\nOnClockChange=yes\nOnTimezoneChange=yes\n";
        let config = parse_timer_from_str(content).unwrap();
        assert!(config.timer.on_clock_change);
        assert!(config.timer.on_timezone_change);
    }

    #[test]
    fn test_clock_change_with_calendar() {
        let content = "[Timer]\nOnCalendar=daily\nOnClockChange=yes\nOnTimezoneChange=no\n";
        let config = parse_timer_from_str(content).unwrap();
        assert!(config.timer.on_clock_change);
        assert!(!config.timer.on_timezone_change);
        assert_eq!(config.timer.on_calendar, vec!["daily".to_owned()]);
    }

    // --- Existing fields still work alongside new ones ---

    #[test]
    fn test_remain_after_elapse_default_with_new_fields() {
        let content = "[Timer]\nOnBootSec=10min\nOnClockChange=yes\n";
        let config = parse_timer_from_str(content).unwrap();
        // RemainAfterElapse defaults to true
        assert!(config.timer.remain_after_elapse);
        assert!(config.timer.on_clock_change);
        assert!(!config.timer.on_timezone_change);
    }

    #[test]
    fn test_full_timer_section() {
        let content = "\
[Timer]
OnBootSec=5min
OnCalendar=hourly
Persistent=yes
WakeSystem=no
RemainAfterElapse=yes
OnClockChange=true
OnTimezoneChange=true
Unit=my-target.service
";
        let config = parse_timer_from_str(content).unwrap();
        assert_eq!(config.timer.on_boot_sec, vec!["5min".to_owned()]);
        assert_eq!(config.timer.on_calendar, vec!["hourly".to_owned()]);
        assert!(config.timer.persistent);
        assert!(!config.timer.wake_system);
        assert!(config.timer.remain_after_elapse);
        assert!(config.timer.on_clock_change);
        assert!(config.timer.on_timezone_change);
        assert_eq!(config.timer.unit, Some("my-target.service".to_owned()));
    }

    #[test]
    fn test_empty_timer_section() {
        let content = "[Timer]\n";
        let config = parse_timer_from_str(content).unwrap();
        assert!(!config.timer.on_clock_change);
        assert!(!config.timer.on_timezone_change);
        assert!(config.timer.remain_after_elapse);
        assert!(config.timer.on_boot_sec.is_empty());
        assert!(config.timer.on_calendar.is_empty());
    }
}

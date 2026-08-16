use log::trace;

use crate::units::{
    ParsedCommonConfig, ParsedDeviceConfig, ParsedFile, ParsingErrorReason, parse_install_section,
    parse_unit_section,
};
use std::path::PathBuf;

/// Parse a `.device` unit file.
///
/// Device units in systemd are special: they are typically auto-generated from
/// udev events (when a device is tagged with `TAG+="systemd"`) rather than
/// loaded from unit files on disk. However, administrators can create `.device`
/// unit files to add `[Unit]` dependencies (e.g., `Wants=`, `After=`) or
/// `[Install]` directives (e.g., `WantedBy=`) for specific devices.
///
/// The `[Device]` section has no directives of its own — all device-specific
/// metadata (`SysFSPath`, `SYSTEMD_ALIAS`, `SYSTEMD_WANTS`, `SYSTEMD_READY`)
/// comes from udev properties at runtime, not from the unit file.
pub fn parse_device(
    parsed_file: ParsedFile,
    path: &PathBuf,
) -> Result<ParsedDeviceConfig, ParsingErrorReason> {
    let mut install_config = None;
    let mut unit_config = None;

    for (name, section) in parsed_file {
        match name.as_str() {
            "[Unit]" => {
                unit_config = Some(parse_unit_section(section)?);
            }
            "[Install]" => {
                install_config = Some(parse_install_section(section)?);
            }
            "[Device]" => {
                // The [Device] section has no directives in systemd.
                // We silently accept it for compatibility but ignore all keys.
                for (key, values) in &section {
                    for (line_num, _value) in values {
                        trace!(
                            "Ignoring [Device] key in {path:?} line {line_num}: {key} (device units have no section directives)"
                        );
                    }
                }
            }
            _ if name.starts_with("[X-") || name.starts_with("[x-") => {
                trace!(
                    "Silently ignoring vendor extension section in device unit {path:?}: {name}"
                );
            }
            _ => {
                trace!("Ignoring unknown section in device unit {path:?}: {name}");
            }
        }
    }

    Ok(ParsedDeviceConfig {
        common: ParsedCommonConfig {
            name: path.file_name().unwrap().to_str().unwrap().to_owned(),
            unit: unit_config.unwrap_or_default(),
            install: install_config.unwrap_or_default(),
            fragment_path: Some(path.clone()),
        },
        sysfs_path: None,
    })
}

/// Create a synthetic (runtime-generated) device unit, as would be created by
/// the manager when udev reports a new device. This is used for:
///
/// 1. Devices tagged with `TAG+="systemd"` in udev rules
/// 2. Devices with `SYSTEMD_ALIAS=` properties that create named device units
/// 3. Placeholder device units created for dependency resolution when a unit
///    has `After=dev-sda1.device` or similar
///
/// The `name` should be the escaped device unit name (e.g., `dev-sda1.device`).
/// The `sysfs_path` is the `/sys/...` path to the device, if known.
pub fn create_synthetic_device_config(
    name: &str,
    sysfs_path: Option<String>,
) -> ParsedDeviceConfig {
    ParsedDeviceConfig {
        common: ParsedCommonConfig {
            name: name.to_owned(),
            unit: Default::default(),
            install: Default::default(),
            fragment_path: None,
        },
        sysfs_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::parse_file;

    fn parse_device_from_str(input: &str) -> ParsedDeviceConfig {
        let parsed = parse_file(input).unwrap();
        parse_device(parsed, &PathBuf::from("dev-sda1.device")).unwrap()
    }

    #[test]
    fn test_empty_device_unit() {
        let conf = parse_device_from_str("");
        assert_eq!(conf.common.name, "dev-sda1.device");
        assert!(conf.sysfs_path.is_none());
        assert!(conf.common.unit.description.is_empty());
    }

    #[test]
    fn test_device_unit_with_unit_section() {
        let conf = parse_device_from_str(
            "[Unit]\n\
             Description=SATA Disk /dev/sda1\n\
             Wants=some.service\n\
             After=some.service\n",
        );
        assert_eq!(conf.common.unit.description, "SATA Disk /dev/sda1");
        assert!(conf.common.unit.wants.iter().any(|w| w == "some.service"));
        assert!(conf.common.unit.after.iter().any(|a| a == "some.service"));
    }

    #[test]
    fn test_device_unit_with_install_section() {
        let conf = parse_device_from_str(
            "[Unit]\n\
             Description=My device\n\
             [Install]\n\
             WantedBy=multi-user.target\n",
        );
        assert_eq!(conf.common.unit.description, "My device");
        assert!(
            conf.common
                .install
                .wanted_by
                .iter()
                .any(|w| w == "multi-user.target")
        );
    }

    #[test]
    fn test_device_unit_with_empty_device_section() {
        // The [Device] section is accepted but has no meaningful directives
        let conf = parse_device_from_str(
            "[Unit]\n\
             Description=Test device\n\
             [Device]\n",
        );
        assert_eq!(conf.common.unit.description, "Test device");
    }

    #[test]
    fn test_device_unit_with_device_section_keys_ignored() {
        // Unknown keys in [Device] should be silently ignored
        let conf = parse_device_from_str(
            "[Unit]\n\
             Description=Test device\n\
             [Device]\n\
             SomeKey=SomeValue\n",
        );
        assert_eq!(conf.common.unit.description, "Test device");
    }

    #[test]
    fn test_device_unit_vendor_extension_ignored() {
        let conf = parse_device_from_str(
            "[Unit]\n\
             Description=Test device\n\
             [X-Custom]\n\
             Foo=Bar\n",
        );
        assert_eq!(conf.common.unit.description, "Test device");
    }

    #[test]
    fn test_device_unit_unknown_section_ignored() {
        let conf = parse_device_from_str(
            "[Unit]\n\
             Description=Test device\n\
             [SomethingElse]\n\
             Foo=Bar\n",
        );
        assert_eq!(conf.common.unit.description, "Test device");
    }

    #[test]
    fn test_device_unit_fragment_path() {
        let parsed = parse_file("[Unit]\nDescription=Test\n").unwrap();
        let path = PathBuf::from("/etc/systemd/system/dev-sda.device");
        let conf = parse_device(parsed, &path).unwrap();
        assert_eq!(
            conf.common.fragment_path,
            Some(PathBuf::from("/etc/systemd/system/dev-sda.device"))
        );
        assert_eq!(conf.common.name, "dev-sda.device");
    }

    #[test]
    fn test_synthetic_device_config_basic() {
        let conf = create_synthetic_device_config("dev-sda1.device", None);
        assert_eq!(conf.common.name, "dev-sda1.device");
        assert!(conf.sysfs_path.is_none());
        assert!(conf.common.fragment_path.is_none());
        assert!(conf.common.unit.description.is_empty());
    }

    #[test]
    fn test_synthetic_device_config_with_sysfs() {
        let conf = create_synthetic_device_config(
            "dev-sda1.device",
            Some("/sys/devices/pci0000:00/0000:00:1f.2/ata1/host0/target0:0:0/0:0:0:0/block/sda/sda1".to_string()),
        );
        assert_eq!(conf.common.name, "dev-sda1.device");
        assert_eq!(
            conf.sysfs_path.as_deref(),
            Some(
                "/sys/devices/pci0000:00/0000:00:1f.2/ata1/host0/target0:0:0/0:0:0:0/block/sda/sda1"
            )
        );
    }

    #[test]
    fn test_synthetic_device_config_no_fragment_path() {
        // Synthetic devices are not loaded from files, so fragment_path is None
        let conf = create_synthetic_device_config("sys-subsystem-net-devices-eth0.device", None);
        assert!(conf.common.fragment_path.is_none());
    }

    #[test]
    fn test_device_unit_with_conditions() {
        let conf = parse_device_from_str(
            "[Unit]\n\
             Description=Conditional device\n\
             ConditionPathExists=/dev/sda1\n",
        );
        assert_eq!(conf.common.unit.description, "Conditional device");
        assert!(!conf.common.unit.conditions.is_empty());
    }

    #[test]
    fn test_device_unit_with_dependencies() {
        let conf = parse_device_from_str(
            "[Unit]\n\
             Description=Device with deps\n\
             Requires=sys-subsystem-net-devices-eth0.device\n\
             Before=network.target\n\
             After=systemd-udevd.service\n",
        );
        assert!(
            conf.common
                .unit
                .requires
                .iter()
                .any(|r| r == "sys-subsystem-net-devices-eth0.device")
        );
        assert!(
            conf.common
                .unit
                .before
                .iter()
                .any(|b| b == "network.target")
        );
        assert!(
            conf.common
                .unit
                .after
                .iter()
                .any(|a| a == "systemd-udevd.service")
        );
    }

    #[test]
    fn test_device_unit_default_dependencies() {
        // Device units should have default_dependencies=true by default
        let conf = parse_device_from_str("[Unit]\nDescription=Test\n");
        assert!(conf.common.unit.default_dependencies);
    }

    #[test]
    fn test_device_unit_no_default_dependencies() {
        let conf = parse_device_from_str(
            "[Unit]\n\
             Description=Test\n\
             DefaultDependencies=no\n",
        );
        assert!(!conf.common.unit.default_dependencies);
    }

    #[test]
    fn test_device_unit_comments_and_blanks() {
        let conf = parse_device_from_str(
            "# This is a device unit\n\
             \n\
             [Unit]\n\
             # Device description\n\
             Description=Test device\n\
             \n",
        );
        assert_eq!(conf.common.unit.description, "Test device");
    }
}

//! systemd-rfkill — Save and restore RF kill switch state across reboots
//!
//! This is a drop-in replacement for systemd-rfkill.service.
//!
//! Usage:
//!   systemd-rfkill save
//!   systemd-rfkill load
//!
//! On `save`, the current soft-block state of all RF kill switches is read from
//! /sys/class/rfkill/rfkillN/soft and saved to /var/lib/systemd/rfkill/.
//!
//! On `load`, the saved soft-block state is read from the state files and written
//! back to the corresponding sysfs entries, restoring the RF state from before reboot.
//!
//! Each rfkill device is identified by its type (e.g. "wlan", "bluetooth") and
//! its persistent device path when available. The state file name encodes both
//! the type and hardware identity so that state is correctly restored even if
//! rfkill indices change across reboots.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

const EXIT_SUCCESS: u8 = 0;
const EXIT_FAILURE: u8 = 1;

/// Base directory for saving rfkill state.
const STATE_DIR: &str = "/var/lib/systemd/rfkill";

/// sysfs base path for rfkill devices.
const SYSFS_RFKILL: &str = "/sys/class/rfkill";

/// systemd-rfkill — Save and restore RF kill switch state across reboots
#[derive(Parser, Debug)]
#[command(name = "systemd-rfkill", version, about)]
struct Cli {
    /// Action to perform: "save" or "load"
    action: String,
}

/// Information about a single rfkill device.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RfkillDevice {
    /// The rfkill index name (e.g. "rfkill0")
    index_name: String,
    /// The sysfs path (e.g. /sys/class/rfkill/rfkill0)
    sysfs_path: PathBuf,
    /// The rfkill type (e.g. "wlan", "bluetooth", "wwan", "gps", "fm", "nfc")
    device_type: String,
    /// The device name (e.g. "phy0", "hci0")
    name: String,
    /// Current soft-block state: true = blocked, false = unblocked
    soft_blocked: bool,
    /// Current hard-block state (informational, we can't change this)
    hard_blocked: bool,
}

impl RfkillDevice {
    /// Read an rfkill device's information from sysfs.
    fn from_sysfs(index_name: &str) -> io::Result<Self> {
        let sysfs_path = PathBuf::from(SYSFS_RFKILL).join(index_name);

        let device_type = read_sysfs_attr(&sysfs_path, "type")?;
        let name = read_sysfs_attr(&sysfs_path, "name").unwrap_or_default();
        let soft = read_sysfs_attr(&sysfs_path, "soft")?;
        let hard = read_sysfs_attr(&sysfs_path, "hard").unwrap_or_else(|_| "0".to_string());

        let soft_blocked = soft.trim() == "1";
        let hard_blocked = hard.trim() == "1";

        Ok(RfkillDevice {
            index_name: index_name.to_string(),
            sysfs_path,
            device_type,
            name,
            soft_blocked,
            hard_blocked,
        })
    }

    /// Compute a stable identifier for this device, used as the state file name.
    /// We use the device type + name to create a persistent identifier that
    /// survives rfkill index renumbering across reboots.
    fn state_id(&self) -> String {
        if self.name.is_empty() {
            // Fall back to type + index if name is not available
            format!("{}:{}", self.device_type, self.index_name)
        } else {
            format!("{}:{}", self.device_type, self.name)
        }
    }

    /// Get the state file path for saving/loading.
    fn state_file_path(&self) -> PathBuf {
        // Replace characters that are problematic in filenames
        let id = self.state_id().replace(['/', '\\'], "-").replace('\0', "");
        PathBuf::from(STATE_DIR).join(id)
    }

    /// Write the soft-block state to sysfs.
    fn set_soft_blocked(&self, blocked: bool) -> io::Result<()> {
        let value = if blocked { "1" } else { "0" };
        let soft_path = self.sysfs_path.join("soft");
        fs::write(&soft_path, value)
    }
}

/// Read a sysfs attribute file and return its trimmed contents.
fn read_sysfs_attr(device_path: &Path, attr: &str) -> io::Result<String> {
    let path = device_path.join(attr);
    let content = fs::read_to_string(&path)?;
    Ok(content.trim().to_string())
}

/// Enumerate all rfkill devices from sysfs.
fn enumerate_rfkill_devices() -> Vec<RfkillDevice> {
    let rfkill_dir = Path::new(SYSFS_RFKILL);
    if !rfkill_dir.is_dir() {
        return Vec::new();
    }

    let mut devices = Vec::new();

    let entries = match fs::read_dir(rfkill_dir) {
        Ok(rd) => rd,
        Err(e) => {
            eprintln!("systemd-rfkill: Failed to read {}: {}", SYSFS_RFKILL, e);
            return devices;
        }
    };

    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.starts_with("rfkill"))
        .collect();
    names.sort();

    for name in &names {
        match RfkillDevice::from_sysfs(name) {
            Ok(dev) => devices.push(dev),
            Err(e) => {
                eprintln!(
                    "systemd-rfkill: Failed to read rfkill device '{}': {}",
                    name, e
                );
            }
        }
    }

    devices
}

/// Save the current soft-block state of all rfkill devices.
fn do_save(verbose: bool) -> u8 {
    let devices = enumerate_rfkill_devices();

    if devices.is_empty() {
        if verbose {
            eprintln!("systemd-rfkill: No rfkill devices found, nothing to save.");
        }
        return EXIT_SUCCESS;
    }

    // Ensure the state directory exists
    if let Err(e) = fs::create_dir_all(STATE_DIR) {
        eprintln!(
            "systemd-rfkill: Failed to create state directory {}: {}",
            STATE_DIR, e
        );
        return EXIT_FAILURE;
    }

    let mut any_failed = false;

    for dev in &devices {
        let state_path = dev.state_file_path();
        let value = if dev.soft_blocked { "1" } else { "0" };

        if verbose {
            eprintln!(
                "systemd-rfkill: Saving {} '{}' ({}): soft_blocked={}",
                dev.device_type, dev.name, dev.index_name, dev.soft_blocked,
            );
        }

        match fs::write(&state_path, format!("{}\n", value)) {
            Ok(()) => {
                if verbose {
                    eprintln!("systemd-rfkill: Saved state to {}", state_path.display());
                }
            }
            Err(e) => {
                eprintln!(
                    "systemd-rfkill: Failed to save state for '{}' to {}: {}",
                    dev.state_id(),
                    state_path.display(),
                    e
                );
                any_failed = true;
            }
        }
    }

    if any_failed {
        EXIT_FAILURE
    } else {
        EXIT_SUCCESS
    }
}

/// Load the saved soft-block state and apply it to all rfkill devices.
fn do_load(verbose: bool) -> u8 {
    let devices = enumerate_rfkill_devices();

    if devices.is_empty() {
        if verbose {
            eprintln!("systemd-rfkill: No rfkill devices found, nothing to load.");
        }
        return EXIT_SUCCESS;
    }

    let state_dir = Path::new(STATE_DIR);
    if !state_dir.is_dir() {
        if verbose {
            eprintln!(
                "systemd-rfkill: State directory {} does not exist, nothing to restore.",
                STATE_DIR
            );
        }
        return EXIT_SUCCESS;
    }

    let mut any_failed = false;

    for dev in &devices {
        let state_path = dev.state_file_path();

        if !state_path.exists() {
            if verbose {
                eprintln!(
                    "systemd-rfkill: No saved state for {} '{}' ({}), skipping.",
                    dev.device_type, dev.name, dev.index_name,
                );
            }
            continue;
        }

        // Don't try to change soft state if hard-blocked
        if dev.hard_blocked {
            if verbose {
                eprintln!(
                    "systemd-rfkill: {} '{}' ({}) is hard-blocked, skipping restore.",
                    dev.device_type, dev.name, dev.index_name,
                );
            }
            continue;
        }

        let saved_state = match fs::read_to_string(&state_path) {
            Ok(content) => content.trim().to_string(),
            Err(e) => {
                eprintln!(
                    "systemd-rfkill: Failed to read saved state from {}: {}",
                    state_path.display(),
                    e
                );
                any_failed = true;
                continue;
            }
        };

        let should_block = match saved_state.as_str() {
            "1" => true,
            "0" => false,
            other => {
                eprintln!(
                    "systemd-rfkill: Invalid saved state '{}' in {}, expected '0' or '1'.",
                    other,
                    state_path.display()
                );
                any_failed = true;
                continue;
            }
        };

        // Skip if the current state already matches
        if dev.soft_blocked == should_block {
            if verbose {
                eprintln!(
                    "systemd-rfkill: {} '{}' ({}) already in desired state (soft_blocked={}).",
                    dev.device_type, dev.name, dev.index_name, should_block,
                );
            }
            continue;
        }

        if verbose {
            eprintln!(
                "systemd-rfkill: Restoring {} '{}' ({}): soft_blocked={} -> {}",
                dev.device_type, dev.name, dev.index_name, dev.soft_blocked, should_block,
            );
        }

        match dev.set_soft_blocked(should_block) {
            Ok(()) => {
                if verbose {
                    eprintln!(
                        "systemd-rfkill: Successfully restored state for {} '{}'.",
                        dev.device_type, dev.name,
                    );
                }
            }
            Err(e) => {
                eprintln!(
                    "systemd-rfkill: Failed to set soft_blocked={} for {} '{}' ({}): {}",
                    should_block, dev.device_type, dev.name, dev.index_name, e,
                );
                any_failed = true;
            }
        }
    }

    if any_failed {
        EXIT_FAILURE
    } else {
        EXIT_SUCCESS
    }
}

fn run() -> u8 {
    let cli = Cli::parse();

    let verbose = std::env::var("SYSTEMD_LOG_LEVEL")
        .map(|v| v == "debug" || v == "info")
        .unwrap_or(false);

    match cli.action.as_str() {
        "save" => do_save(verbose),
        "load" => do_load(verbose),
        other => {
            eprintln!(
                "systemd-rfkill: Unknown action '{}'. Expected 'save' or 'load'.",
                other
            );
            EXIT_FAILURE
        }
    }
}

fn main() -> ExitCode {
    ExitCode::from(run())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_id_with_name() {
        let dev = RfkillDevice {
            index_name: "rfkill0".to_string(),
            sysfs_path: PathBuf::from("/sys/class/rfkill/rfkill0"),
            device_type: "wlan".to_string(),
            name: "phy0".to_string(),
            soft_blocked: false,
            hard_blocked: false,
        };
        assert_eq!(dev.state_id(), "wlan:phy0");
    }

    #[test]
    fn test_state_id_without_name() {
        let dev = RfkillDevice {
            index_name: "rfkill1".to_string(),
            sysfs_path: PathBuf::from("/sys/class/rfkill/rfkill1"),
            device_type: "bluetooth".to_string(),
            name: "".to_string(),
            soft_blocked: false,
            hard_blocked: false,
        };
        assert_eq!(dev.state_id(), "bluetooth:rfkill1");
    }

    #[test]
    fn test_state_file_path() {
        let dev = RfkillDevice {
            index_name: "rfkill0".to_string(),
            sysfs_path: PathBuf::from("/sys/class/rfkill/rfkill0"),
            device_type: "wlan".to_string(),
            name: "phy0".to_string(),
            soft_blocked: false,
            hard_blocked: false,
        };
        assert_eq!(
            dev.state_file_path(),
            PathBuf::from("/var/lib/systemd/rfkill/wlan:phy0")
        );
    }

    #[test]
    fn test_state_file_path_special_chars() {
        let dev = RfkillDevice {
            index_name: "rfkill2".to_string(),
            sysfs_path: PathBuf::from("/sys/class/rfkill/rfkill2"),
            device_type: "wwan".to_string(),
            name: "device/with/slashes".to_string(),
            soft_blocked: true,
            hard_blocked: false,
        };
        assert_eq!(
            dev.state_file_path(),
            PathBuf::from("/var/lib/systemd/rfkill/wwan:device-with-slashes")
        );
    }

    #[test]
    fn test_enumerate_no_crash() {
        // Just ensure enumerate doesn't panic even if /sys/class/rfkill doesn't exist
        let _devices = enumerate_rfkill_devices();
    }

    #[test]
    fn test_rfkill_device_equality() {
        let dev1 = RfkillDevice {
            index_name: "rfkill0".to_string(),
            sysfs_path: PathBuf::from("/sys/class/rfkill/rfkill0"),
            device_type: "wlan".to_string(),
            name: "phy0".to_string(),
            soft_blocked: false,
            hard_blocked: false,
        };
        let dev2 = dev1.clone();
        assert_eq!(dev1, dev2);
    }

    #[test]
    fn test_rfkill_device_different_state() {
        let dev1 = RfkillDevice {
            index_name: "rfkill0".to_string(),
            sysfs_path: PathBuf::from("/sys/class/rfkill/rfkill0"),
            device_type: "wlan".to_string(),
            name: "phy0".to_string(),
            soft_blocked: false,
            hard_blocked: false,
        };
        let dev2 = RfkillDevice {
            soft_blocked: true,
            ..dev1.clone()
        };
        assert_ne!(dev1, dev2);
    }

    #[test]
    fn test_state_roundtrip() {
        // Test that we can write and read a state value
        let dir = std::env::temp_dir().join("systemd-rfkill-test-roundtrip");
        let _ = fs::create_dir_all(&dir);
        let state_file = dir.join("test-state");

        // Save blocked state
        fs::write(&state_file, "1\n").unwrap();
        let content = fs::read_to_string(&state_file).unwrap();
        assert_eq!(content.trim(), "1");

        // Save unblocked state
        fs::write(&state_file, "0\n").unwrap();
        let content = fs::read_to_string(&state_file).unwrap();
        assert_eq!(content.trim(), "0");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_from_sysfs_nonexistent() {
        let result = RfkillDevice::from_sysfs("rfkill_nonexistent_99999");
        assert!(result.is_err());
    }

    #[test]
    fn test_do_save_no_devices() {
        // On a system with no rfkill devices (like a VM), save should succeed
        // This test may or may not find devices depending on the test environment
        // but it should never panic
        let _result = do_save(false);
    }

    #[test]
    fn test_do_load_no_state_dir() {
        // Loading when there's no state directory should succeed (nothing to restore)
        // This is a no-op test that verifies no panic occurs
        let _result = do_load(false);
    }

    #[test]
    fn test_state_id_deterministic() {
        let dev = RfkillDevice {
            index_name: "rfkill0".to_string(),
            sysfs_path: PathBuf::from("/sys/class/rfkill/rfkill0"),
            device_type: "bluetooth".to_string(),
            name: "hci0".to_string(),
            soft_blocked: false,
            hard_blocked: false,
        };
        // Calling state_id multiple times should return the same result
        assert_eq!(dev.state_id(), dev.state_id());
        assert_eq!(dev.state_file_path(), dev.state_file_path());
    }
}

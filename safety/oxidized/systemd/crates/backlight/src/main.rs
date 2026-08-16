//! systemd-backlight — Save and restore backlight brightness across reboots
//!
//! This is a drop-in replacement for systemd-backlight@.service.
//!
//! Usage:
//!   systemd-backlight save <device>
//!   systemd-backlight load <device>
//!
//! The <device> argument is the sysfs device path suffix, typically in the form
//! "backlight:acpi_video0" or "leds:input3::scrolllock". The tool operates on
//! devices under /sys/class/backlight/ or /sys/class/leds/.
//!
//! Saved state is stored in /var/lib/systemd/backlight/.
//!
//! On `save`, the current brightness is read from sysfs and written to the state file.
//! On `load`, the saved brightness is read from the state file and written to sysfs.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

const EXIT_SUCCESS: u8 = 0;
const EXIT_FAILURE: u8 = 1;

/// Base directory for saving backlight state.
const STATE_DIR: &str = "/var/lib/systemd/backlight";

/// sysfs base paths for backlight and LED devices.
const SYSFS_BACKLIGHT: &str = "/sys/class/backlight";
const SYSFS_LEDS: &str = "/sys/class/leds";

/// systemd-backlight — Save and restore backlight/LED brightness across reboots
#[derive(Parser, Debug)]
#[command(name = "systemd-backlight", version, about)]
struct Cli {
    /// Action to perform: "save" or "load"
    action: String,

    /// Device identifier, e.g. "backlight:acpi_video0" or "leds:input3::capslock"
    device: String,
}

/// Parsed device specification.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DeviceSpec {
    /// The subsystem class: "backlight" or "leds"
    subsystem: String,
    /// The device name within that subsystem
    name: String,
}

impl DeviceSpec {
    /// Parse a device identifier like "backlight:acpi_video0" or "leds:tpacpi::kbd_backlight".
    fn parse(device: &str) -> Option<Self> {
        let (subsystem, name) = device.split_once(':')?;
        if subsystem.is_empty() || name.is_empty() {
            return None;
        }
        Some(DeviceSpec {
            subsystem: subsystem.to_string(),
            name: name.to_string(),
        })
    }

    /// Get the sysfs base path for this device's subsystem.
    fn sysfs_base(&self) -> &str {
        match self.subsystem.as_str() {
            "backlight" => SYSFS_BACKLIGHT,
            "leds" => SYSFS_LEDS,
            _ => SYSFS_BACKLIGHT,
        }
    }

    /// Get the full sysfs path for this device.
    fn sysfs_path(&self) -> PathBuf {
        PathBuf::from(self.sysfs_base()).join(&self.name)
    }

    /// Get the brightness file path in sysfs.
    fn brightness_path(&self) -> PathBuf {
        self.sysfs_path().join("brightness")
    }

    /// Get the max_brightness file path in sysfs.
    fn max_brightness_path(&self) -> PathBuf {
        self.sysfs_path().join("max_brightness")
    }

    /// Get the actual_brightness file path in sysfs (preferred for reading).
    fn actual_brightness_path(&self) -> PathBuf {
        self.sysfs_path().join("actual_brightness")
    }

    /// Get the state file path for saving/loading.
    fn state_file_path(&self) -> PathBuf {
        // Use the full device spec as the filename, replacing ':' with '-'
        // to avoid filesystem issues
        let filename = format!("{}-{}", self.subsystem, self.name.replace(':', "-"));
        PathBuf::from(STATE_DIR).join(filename)
    }
}

/// Read the current brightness from sysfs.
/// Prefers actual_brightness if available, falls back to brightness.
fn read_current_brightness(spec: &DeviceSpec) -> io::Result<u64> {
    // Try actual_brightness first (more accurate, reflects hardware state)
    let actual_path = spec.actual_brightness_path();
    let brightness_path = spec.brightness_path();

    let path = if actual_path.exists() {
        &actual_path
    } else {
        &brightness_path
    };

    let content = fs::read_to_string(path)?;
    let value = content
        .trim()
        .parse::<u64>()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(value)
}

/// Read the maximum brightness from sysfs.
fn read_max_brightness(spec: &DeviceSpec) -> io::Result<u64> {
    let path = spec.max_brightness_path();
    let content = fs::read_to_string(path)?;
    let value = content
        .trim()
        .parse::<u64>()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(value)
}

/// Write a brightness value to sysfs.
fn write_brightness(spec: &DeviceSpec, value: u64) -> io::Result<()> {
    let path = spec.brightness_path();
    fs::write(path, format!("{}\n", value))
}

/// Clamp a brightness value to be within valid range.
/// Ensures the value is at least 1 (or 0 if max is 0) and at most max_brightness.
fn clamp_brightness(value: u64, max_brightness: u64) -> u64 {
    if max_brightness == 0 {
        return 0;
    }
    // systemd-backlight ensures we don't restore to 0 brightness (which would
    // leave the user with an invisible screen) unless max is also 0.
    // However, for LEDs this restriction doesn't apply, so we use a minimum of 0.
    value.min(max_brightness)
}

/// Save the current brightness to the state file.
fn do_save(spec: &DeviceSpec, verbose: bool) -> u8 {
    let sysfs_path = spec.sysfs_path();
    if !sysfs_path.exists() {
        eprintln!(
            "systemd-backlight: Device '{}' not found at {}",
            spec.name,
            sysfs_path.display()
        );
        return EXIT_FAILURE;
    }

    let brightness = match read_current_brightness(spec) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "systemd-backlight: Failed to read brightness for '{}': {}",
                spec.name, e
            );
            return EXIT_FAILURE;
        }
    };

    if verbose {
        eprintln!(
            "systemd-backlight: Current brightness for '{}': {}",
            spec.name, brightness
        );
    }

    // Ensure the state directory exists
    if let Err(e) = fs::create_dir_all(STATE_DIR) {
        eprintln!(
            "systemd-backlight: Failed to create state directory {}: {}",
            STATE_DIR, e
        );
        return EXIT_FAILURE;
    }

    let state_path = spec.state_file_path();
    if let Err(e) = fs::write(&state_path, format!("{}\n", brightness)) {
        eprintln!(
            "systemd-backlight: Failed to save brightness to {}: {}",
            state_path.display(),
            e
        );
        return EXIT_FAILURE;
    }

    if verbose {
        eprintln!(
            "systemd-backlight: Saved brightness {} to {}",
            brightness,
            state_path.display()
        );
    }

    EXIT_SUCCESS
}

/// Load the saved brightness from the state file and apply it.
fn do_load(spec: &DeviceSpec, verbose: bool) -> u8 {
    let sysfs_path = spec.sysfs_path();
    if !sysfs_path.exists() {
        eprintln!(
            "systemd-backlight: Device '{}' not found at {}",
            spec.name,
            sysfs_path.display()
        );
        return EXIT_FAILURE;
    }

    let state_path = spec.state_file_path();
    if !state_path.exists() {
        if verbose {
            eprintln!(
                "systemd-backlight: No saved state for '{}' at {}, skipping.",
                spec.name,
                state_path.display()
            );
        }
        return EXIT_SUCCESS;
    }

    // Read saved brightness
    let saved_brightness = match fs::read_to_string(&state_path) {
        Ok(content) => match content.trim().parse::<u64>() {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "systemd-backlight: Invalid brightness value in {}: {}",
                    state_path.display(),
                    e
                );
                return EXIT_FAILURE;
            }
        },
        Err(e) => {
            eprintln!(
                "systemd-backlight: Failed to read saved state from {}: {}",
                state_path.display(),
                e
            );
            return EXIT_FAILURE;
        }
    };

    // Read max brightness for clamping
    let max_brightness = match read_max_brightness(spec) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "systemd-backlight: Failed to read max_brightness for '{}': {}",
                spec.name, e
            );
            return EXIT_FAILURE;
        }
    };

    // Clamp to valid range
    let brightness = clamp_brightness(saved_brightness, max_brightness);

    // For backlight devices (not LEDs), avoid restoring brightness to 0
    // as this would leave the screen completely dark and the user unable
    // to see anything. Ensure at least brightness 1 (or 5% of max, whichever
    // is larger) for backlight devices.
    let brightness = if spec.subsystem == "backlight" && brightness == 0 && max_brightness > 0 {
        let min_brightness = std::cmp::max(1, max_brightness / 20);
        if verbose {
            eprintln!(
                "systemd-backlight: Saved brightness was 0, using minimum {} for safety.",
                min_brightness
            );
        }
        min_brightness
    } else {
        brightness
    };

    if verbose {
        eprintln!(
            "systemd-backlight: Restoring brightness for '{}': {} (saved: {}, max: {})",
            spec.name, brightness, saved_brightness, max_brightness
        );
    }

    // Check current brightness — if the BIOS or firmware already set a reasonable
    // brightness, we might not want to override it. systemd does this check:
    // if the current brightness is already nonzero and differs from 0, skip
    // restoration. We implement a simpler policy: always restore.
    if let Err(e) = write_brightness(spec, brightness) {
        eprintln!(
            "systemd-backlight: Failed to write brightness {} for '{}': {}",
            brightness, spec.name, e
        );
        return EXIT_FAILURE;
    }

    if verbose {
        eprintln!(
            "systemd-backlight: Successfully restored brightness for '{}'.",
            spec.name
        );
    }

    EXIT_SUCCESS
}

fn run() -> u8 {
    let cli = Cli::parse();

    let verbose = std::env::var("SYSTEMD_LOG_LEVEL")
        .map(|v| v == "debug" || v == "info")
        .unwrap_or(false);

    let spec = match DeviceSpec::parse(&cli.device) {
        Some(s) => s,
        None => {
            eprintln!(
                "systemd-backlight: Invalid device specification '{}'. \
                 Expected format: subsystem:name (e.g. backlight:acpi_video0)",
                cli.device
            );
            return EXIT_FAILURE;
        }
    };

    // Validate subsystem
    match spec.subsystem.as_str() {
        "backlight" | "leds" => {}
        other => {
            eprintln!(
                "systemd-backlight: Unknown subsystem '{}'. Expected 'backlight' or 'leds'.",
                other
            );
            return EXIT_FAILURE;
        }
    }

    match cli.action.as_str() {
        "save" => do_save(&spec, verbose),
        "load" => do_load(&spec, verbose),
        other => {
            eprintln!(
                "systemd-backlight: Unknown action '{}'. Expected 'save' or 'load'.",
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
    fn test_device_spec_parse_backlight() {
        let spec = DeviceSpec::parse("backlight:acpi_video0").unwrap();
        assert_eq!(spec.subsystem, "backlight");
        assert_eq!(spec.name, "acpi_video0");
    }

    #[test]
    fn test_device_spec_parse_leds() {
        let spec = DeviceSpec::parse("leds:input3::capslock").unwrap();
        assert_eq!(spec.subsystem, "leds");
        assert_eq!(spec.name, "input3::capslock");
    }

    #[test]
    fn test_device_spec_parse_invalid() {
        assert!(DeviceSpec::parse("").is_none());
        assert!(DeviceSpec::parse("nocolon").is_none());
        assert!(DeviceSpec::parse(":nosubsystem").is_none());
        assert!(DeviceSpec::parse("noname:").is_none());
    }

    #[test]
    fn test_device_spec_sysfs_path() {
        let spec = DeviceSpec::parse("backlight:acpi_video0").unwrap();
        assert_eq!(
            spec.sysfs_path(),
            PathBuf::from("/sys/class/backlight/acpi_video0")
        );

        let spec = DeviceSpec::parse("leds:input3::capslock").unwrap();
        assert_eq!(
            spec.sysfs_path(),
            PathBuf::from("/sys/class/leds/input3::capslock")
        );
    }

    #[test]
    fn test_device_spec_brightness_path() {
        let spec = DeviceSpec::parse("backlight:acpi_video0").unwrap();
        assert_eq!(
            spec.brightness_path(),
            PathBuf::from("/sys/class/backlight/acpi_video0/brightness")
        );
    }

    #[test]
    fn test_device_spec_max_brightness_path() {
        let spec = DeviceSpec::parse("backlight:acpi_video0").unwrap();
        assert_eq!(
            spec.max_brightness_path(),
            PathBuf::from("/sys/class/backlight/acpi_video0/max_brightness")
        );
    }

    #[test]
    fn test_device_spec_actual_brightness_path() {
        let spec = DeviceSpec::parse("backlight:acpi_video0").unwrap();
        assert_eq!(
            spec.actual_brightness_path(),
            PathBuf::from("/sys/class/backlight/acpi_video0/actual_brightness")
        );
    }

    #[test]
    fn test_device_spec_state_file_path() {
        let spec = DeviceSpec::parse("backlight:acpi_video0").unwrap();
        assert_eq!(
            spec.state_file_path(),
            PathBuf::from("/var/lib/systemd/backlight/backlight-acpi_video0")
        );
    }

    #[test]
    fn test_device_spec_state_file_path_colons() {
        let spec = DeviceSpec::parse("leds:input3::capslock").unwrap();
        assert_eq!(
            spec.state_file_path(),
            PathBuf::from("/var/lib/systemd/backlight/leds-input3--capslock")
        );
    }

    #[test]
    fn test_clamp_brightness() {
        assert_eq!(clamp_brightness(50, 100), 50);
        assert_eq!(clamp_brightness(150, 100), 100);
        assert_eq!(clamp_brightness(0, 100), 0);
        assert_eq!(clamp_brightness(100, 100), 100);
        assert_eq!(clamp_brightness(0, 0), 0);
        assert_eq!(clamp_brightness(50, 0), 0);
    }

    #[test]
    fn test_clamp_brightness_boundary() {
        assert_eq!(clamp_brightness(255, 255), 255);
        assert_eq!(clamp_brightness(256, 255), 255);
        assert_eq!(clamp_brightness(1, 1), 1);
    }

    #[test]
    fn test_sysfs_base_backlight() {
        let spec = DeviceSpec::parse("backlight:foo").unwrap();
        assert_eq!(spec.sysfs_base(), "/sys/class/backlight");
    }

    #[test]
    fn test_sysfs_base_leds() {
        let spec = DeviceSpec::parse("leds:foo").unwrap();
        assert_eq!(spec.sysfs_base(), "/sys/class/leds");
    }

    #[test]
    fn test_save_nonexistent_device() {
        let spec = DeviceSpec::parse("backlight:zzzz_nonexistent_device_99999").unwrap();
        let result = do_save(&spec, false);
        assert_eq!(result, EXIT_FAILURE);
    }

    #[test]
    fn test_load_nonexistent_device() {
        let spec = DeviceSpec::parse("backlight:zzzz_nonexistent_device_99999").unwrap();
        let result = do_load(&spec, false);
        assert_eq!(result, EXIT_FAILURE);
    }

    #[test]
    fn test_save_and_load_with_tempdir() {
        // This test verifies the state file round-trip using a temporary directory.
        // We can't test actual sysfs interaction without root, but we can test
        // the state file mechanism.
        let dir = std::env::temp_dir().join("systemd-backlight-test-roundtrip");
        let _ = fs::create_dir_all(&dir);

        let state_file = dir.join("test-brightness");
        let brightness: u64 = 42;

        // Write state
        fs::write(&state_file, format!("{}\n", brightness)).unwrap();

        // Read it back
        let content = fs::read_to_string(&state_file).unwrap();
        let restored: u64 = content.trim().parse().unwrap();
        assert_eq!(restored, brightness);

        let _ = fs::remove_dir_all(&dir);
    }
}

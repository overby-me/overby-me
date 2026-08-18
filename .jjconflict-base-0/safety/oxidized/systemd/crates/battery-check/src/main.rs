//! `systemd-battery-check` - check battery level to see whether there is
//! enough charge to continue booting.
//!
//! A faithful port of upstream `src/battery-check/battery-check.c` plus the
//! `battery_is_discharging_and_low` / `on_ac_power` helpers from
//! `src/shared/battery-util.c`. On a system that is on AC power or has no (low)
//! battery it exits 0; on a critically low battery it warns on the console and,
//! after a grace period, exits non-zero to trigger power-off.

use std::path::Path;
use std::process::ExitCode;

const BATTERY_LOW_CAPACITY_LEVEL: i64 = 5;
const BATTERY_LOW_MESSAGE: &str = "Battery level critically low. Please connect your charger or the system will power off in 10 seconds.";
const BATTERY_RESTORED_MESSAGE: &str = "A.C. power restored, continuing.";

const POWER_SUPPLY_DIR: &str = "/sys/class/power_supply";

fn help() {
    println!(
        "systemd-battery-check\n\n\
         Check battery level to see whether there's enough charge.\n\n   \
         -h --help            Show this help\n      \
         --version         Show package version"
    );
}

fn version() {
    println!(
        "systemd {} (systemd-battery-check)",
        env!("CARGO_PKG_VERSION")
    );
}

/// Read a trimmed sysfs attribute.
fn read_attr(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name))
        .ok()
        .map(|s| s.trim().to_string())
}

/// Whether the system is running on AC power. Mirrors systemd's `on_ac_power()`
/// closely enough for the boot check: true if any Mains-type supply is online,
/// false if Mains supplies exist but are all offline, and true when there is no
/// AC adapter at all (desktop/VM assumption).
fn on_ac_power() -> bool {
    let rd = match std::fs::read_dir(POWER_SUPPLY_DIR) {
        Ok(rd) => rd,
        Err(_) => return true, // no power-supply subsystem at all -> assume AC
    };

    let mut found_mains = false;
    let mut any_online = false;
    for ent in rd.flatten() {
        let dir = ent.path();
        let typ = read_attr(&dir, "type").unwrap_or_default();
        // Mains / USB / Wireless / BrickID are all "AC-like" supplies.
        let is_ac = matches!(
            typ.as_str(),
            "Mains"
                | "USB"
                | "USB_ACA"
                | "USB_C"
                | "USB_PD"
                | "USB_PD_DRP"
                | "Wireless"
                | "BrickID"
        );
        if !is_ac {
            continue;
        }
        // Skip supplies that report a scope of "Device" (peripheral batteries).
        if read_attr(&dir, "scope").as_deref() == Some("Device") {
            continue;
        }
        found_mains = true;
        if read_attr(&dir, "online").as_deref() == Some("1") {
            any_online = true;
        }
    }

    if any_online {
        return true;
    }
    if found_mains {
        return false; // AC adapters present but all offline
    }
    true // no AC adapter found -> assume AC
}

/// Read a battery's charge as a percentage, mirroring
/// `battery_read_capacity_percentage()`. Returns `None` when it can't be read.
fn battery_read_capacity_percentage(dir: &Path) -> Option<i64> {
    let cap = read_attr(dir, "capacity")?;
    let level: i64 = cap.parse().ok()?;
    if !(0..=100).contains(&level) {
        return None;
    }
    Some(level)
}

/// Whether the system is discharging and the battery is critically low. Mirrors
/// `battery_is_discharging_and_low()`: on AC -> not low; otherwise a battery is
/// "low" only if every readable battery is at or below the threshold and none
/// is charged or unreadable.
fn battery_is_discharging_and_low() -> Result<bool, String> {
    if on_ac_power() {
        return Ok(false);
    }

    let rd = match std::fs::read_dir(POWER_SUPPLY_DIR) {
        Ok(rd) => rd,
        Err(e) => return Err(format!("Failed to enumerate power supplies: {e}")),
    };

    let mut unsure = false;
    let mut found_low = false;
    for ent in rd.flatten() {
        let dir = ent.path();
        if read_attr(&dir, "type").as_deref() != Some("Battery") {
            continue;
        }
        if read_attr(&dir, "scope").as_deref() == Some("Device") {
            continue;
        }
        match battery_read_capacity_percentage(&dir) {
            None => {
                unsure = true;
            }
            Some(level) if level > BATTERY_LOW_CAPACITY_LEVEL => {
                return Ok(false); // found a charged battery
            }
            Some(_) => {
                found_low = true;
            }
        }
    }

    if unsure {
        return Ok(false);
    }
    Ok(found_low)
}

/// Parse `systemd.battery_check=BOOL` from the kernel command line. Missing ->
/// enabled (true), matching `PROC_CMDLINE_TRUE_WHEN_MISSING`.
fn battery_check_enabled() -> bool {
    let cmdline = match std::fs::read_to_string("/proc/cmdline") {
        Ok(c) => c,
        Err(_) => return true,
    };
    for tok in cmdline.split_whitespace() {
        let tok = tok.strip_prefix("rd.").unwrap_or(tok);
        if let Some(val) = tok.strip_prefix("systemd.battery_check=") {
            return parse_bool(val).unwrap_or(true);
        }
        if tok == "systemd.battery_check" {
            return true;
        }
    }
    true
}

fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "1" | "yes" | "y" | "true" | "t" | "on" => Some(true),
        "0" | "no" | "n" | "false" | "f" | "off" => Some(false),
        _ => None,
    }
}

fn write_console(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open("/dev/console") {
        let _ = writeln!(f, "{msg}");
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        help();
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--version") {
        version();
        return ExitCode::SUCCESS;
    }
    if let Some(bad) = args.first() {
        eprintln!("systemd-battery-check takes no argument: {bad}");
        return ExitCode::from(1);
    }

    if !battery_check_enabled() {
        println!(
            "Checking battery status and AC power existence is disabled by the kernel command line, skipping execution."
        );
        return ExitCode::SUCCESS;
    }

    match battery_is_discharging_and_low() {
        Err(e) => {
            eprintln!("Failed to check battery status, ignoring: {e}");
            return ExitCode::SUCCESS;
        }
        Ok(false) => return ExitCode::SUCCESS,
        Ok(true) => {}
    }

    // Critically low battery: warn, wait for a grace period, then re-check.
    eprintln!("{BATTERY_LOW_MESSAGE}");
    write_console(BATTERY_LOW_MESSAGE);

    std::thread::sleep(std::time::Duration::from_secs(10));

    match battery_is_discharging_and_low() {
        Err(e) => {
            eprintln!(
                "Failed to check battery status, assuming not charged yet, powering off: {e}"
            );
            ExitCode::from(1)
        }
        Ok(true) => {
            eprintln!("Battery level critically low, powering off.");
            ExitCode::from(1)
        }
        Ok(false) => {
            println!("{BATTERY_RESTORED_MESSAGE}");
            write_console(BATTERY_RESTORED_MESSAGE);
            ExitCode::SUCCESS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_parsing() {
        assert_eq!(parse_bool("1"), Some(true));
        assert_eq!(parse_bool("off"), Some(false));
        assert_eq!(parse_bool("maybe"), None);
    }

    #[test]
    fn no_power_supply_is_ac() {
        // With no readable /sys/class/power_supply the tool assumes AC and is
        // therefore never "discharging and low".
        if !Path::new(POWER_SUPPLY_DIR).exists() {
            assert!(on_ac_power());
            assert_eq!(battery_is_discharging_and_low().ok(), Some(false));
        }
    }
}

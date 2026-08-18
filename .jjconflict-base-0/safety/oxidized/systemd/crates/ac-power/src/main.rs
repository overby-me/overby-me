//! systemd-ac-power — Report whether the system is running on AC power.
//!
//! A drop-in replacement for `systemd-ac-power(1)`. Checks the power
//! supply subsystem via `/sys/class/power_supply/` and reports whether
//! the system is currently connected to an AC (mains) power source.
//!
//! Exit codes:
//!   0 — The system is on AC power (or no battery was found, implying
//!       a desktop/server that is always on mains power).
//!   1 — The system is running on battery power.
//!
//! Options:
//!   -v, --verbose  Print a human-readable status message.
//!   -q, --quiet    Suppress all output (default behavior; kept for compat).

use clap::Parser;
use std::fs;
use std::path::Path;
use std::process;

#[derive(Parser, Debug)]
#[command(
    name = "systemd-ac-power",
    about = "Report whether the system is on AC (mains) power",
    version
)]
struct Cli {
    /// Print a human-readable status message.
    #[arg(short, long)]
    verbose: bool,

    /// Suppress all output (this is the default; kept for compatibility).
    #[arg(short, long)]
    quiet: bool,

    /// Check whether the system has a battery (exit 0 if a battery is
    /// present, 1 otherwise). This matches `--check-capacity` semantics
    /// in newer systemd versions.
    #[arg(long)]
    check_capacity: bool,

    /// Low battery threshold percentage. When combined with
    /// `--check-capacity`, exits 0 only if battery level is above this
    /// threshold.
    #[arg(long, value_name = "PERCENT")]
    low: Option<u32>,
}

/// Represents the type of a power supply as reported by the kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupplyType {
    Mains,
    Battery,
    Ups,
    Usb,
    Unknown,
}

impl SupplyType {
    fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "mains" => SupplyType::Mains,
            "battery" => SupplyType::Battery,
            "ups" => SupplyType::Ups,
            "usb" => SupplyType::Usb,
            _ => SupplyType::Unknown,
        }
    }
}

/// Read a sysfs attribute file, returning the trimmed contents.
fn read_sysfs(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Describes the state of a single power supply entry.
#[derive(Debug)]
struct PowerSupply {
    name: String,
    supply_type: SupplyType,
    online: Option<bool>,
    status: Option<String>,
    capacity: Option<u32>,
}

/// Enumerate all power supply devices from `/sys/class/power_supply/`.
fn enumerate_power_supplies() -> Vec<PowerSupply> {
    let base = Path::new("/sys/class/power_supply");
    let mut supplies = Vec::new();

    let entries = match fs::read_dir(base) {
        Ok(e) => e,
        Err(_) => return supplies,
    };

    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();

        let supply_type = read_sysfs(&dir.join("type"))
            .map(|s| SupplyType::from_str(&s))
            .unwrap_or(SupplyType::Unknown);

        let online = read_sysfs(&dir.join("online")).and_then(|s| match s.as_str() {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        });

        let status = read_sysfs(&dir.join("status"));

        let capacity = read_sysfs(&dir.join("capacity")).and_then(|s| s.parse::<u32>().ok());

        supplies.push(PowerSupply {
            name,
            supply_type,
            online,
            status,
            capacity,
        });
    }

    supplies
}

/// Determine whether the system is on AC power.
///
/// The logic mirrors what real systemd does:
///
/// 1. If any mains/UPS/USB supply is online → on AC power.
/// 2. If any battery is found and is discharging → on battery.
/// 3. If no power supply information is available at all (e.g. a desktop
///    without ACPI battery support), assume AC power.
fn is_on_ac_power(supplies: &[PowerSupply]) -> bool {
    let mut found_any_supply = false;
    let mut found_offline_mains = false;
    let mut found_discharging_battery = false;

    for supply in supplies {
        match supply.supply_type {
            SupplyType::Mains | SupplyType::Ups | SupplyType::Usb => {
                found_any_supply = true;
                if supply.online == Some(true) {
                    return true;
                }
                found_offline_mains = true;
            }
            SupplyType::Battery => {
                found_any_supply = true;
                if let Some(ref status) = supply.status {
                    let s = status.to_lowercase();
                    if s == "discharging" {
                        found_discharging_battery = true;
                    }
                    // "Charging" or "Full" implies AC is connected
                    if s == "charging" || s == "full" || s == "not charging" {
                        return true;
                    }
                }
            }
            SupplyType::Unknown => {
                // Ignore unknown supply types for the AC determination.
            }
        }
    }

    if found_discharging_battery {
        return false;
    }

    if found_offline_mains {
        return false;
    }

    // No supplies found at all → assume AC (desktop / server).
    // Or we found supplies but none were conclusively offline or discharging.
    if !found_any_supply {
        return true;
    }

    // Fallback: if we have supplies but couldn't determine state, assume AC.
    true
}

/// Check whether the system has a battery.
fn has_battery(supplies: &[PowerSupply]) -> bool {
    supplies
        .iter()
        .any(|s| s.supply_type == SupplyType::Battery)
}

/// Get the minimum battery capacity across all batteries, or `None` if
/// no battery reports a capacity.
fn min_battery_capacity(supplies: &[PowerSupply]) -> Option<u32> {
    supplies
        .iter()
        .filter(|s| s.supply_type == SupplyType::Battery)
        .filter_map(|s| s.capacity)
        .min()
}

fn main() {
    let cli = Cli::parse();

    let supplies = enumerate_power_supplies();

    if cli.check_capacity {
        let has_bat = has_battery(&supplies);

        if !has_bat {
            if cli.verbose {
                println!("No battery found.");
            }
            process::exit(1);
        }

        if let Some(threshold) = cli.low {
            let cap = min_battery_capacity(&supplies);
            match cap {
                Some(c) if c > threshold => {
                    if cli.verbose {
                        println!("Battery capacity {c}% is above threshold {threshold}%.");
                    }
                    process::exit(0);
                }
                Some(c) => {
                    if cli.verbose {
                        println!("Battery capacity {c}% is at or below threshold {threshold}%.");
                    }
                    process::exit(1);
                }
                None => {
                    if cli.verbose {
                        println!("Battery found but capacity is unknown.");
                    }
                    process::exit(1);
                }
            }
        }

        if cli.verbose {
            if let Some(cap) = min_battery_capacity(&supplies) {
                println!("Battery present, capacity {cap}%.");
            } else {
                println!("Battery present, capacity unknown.");
            }
        }
        process::exit(0);
    }

    let on_ac = is_on_ac_power(&supplies);

    if cli.verbose {
        if on_ac {
            println!("on-line");
        } else {
            println!("off-line");
        }

        // Print details about discovered supplies
        for supply in &supplies {
            let type_str = match supply.supply_type {
                SupplyType::Mains => "Mains",
                SupplyType::Battery => "Battery",
                SupplyType::Ups => "UPS",
                SupplyType::Usb => "USB",
                SupplyType::Unknown => "Unknown",
            };

            let online_str = match supply.online {
                Some(true) => "online",
                Some(false) => "offline",
                None => "unknown",
            };

            let status_str = supply.status.as_deref().unwrap_or("n/a");

            let cap_str = supply
                .capacity
                .map(|c| format!("{c}%"))
                .unwrap_or_else(|| "n/a".to_string());

            eprintln!(
                "  {}: type={}, online={}, status={}, capacity={}",
                supply.name, type_str, online_str, status_str, cap_str
            );
        }
    }

    if on_ac {
        process::exit(0);
    } else {
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_supply(
        name: &str,
        supply_type: SupplyType,
        online: Option<bool>,
        status: Option<&str>,
        capacity: Option<u32>,
    ) -> PowerSupply {
        PowerSupply {
            name: name.to_string(),
            supply_type,
            online,
            status: status.map(String::from),
            capacity,
        }
    }

    #[test]
    fn test_no_supplies_means_ac() {
        assert!(is_on_ac_power(&[]));
    }

    #[test]
    fn test_mains_online() {
        let supplies = vec![make_supply(
            "AC0",
            SupplyType::Mains,
            Some(true),
            None,
            None,
        )];
        assert!(is_on_ac_power(&supplies));
    }

    #[test]
    fn test_mains_offline_battery_discharging() {
        let supplies = vec![
            make_supply("AC0", SupplyType::Mains, Some(false), None, None),
            make_supply(
                "BAT0",
                SupplyType::Battery,
                None,
                Some("Discharging"),
                Some(72),
            ),
        ];
        assert!(!is_on_ac_power(&supplies));
    }

    #[test]
    fn test_mains_offline_battery_charging() {
        let supplies = vec![
            make_supply("AC0", SupplyType::Mains, Some(false), None, None),
            make_supply(
                "BAT0",
                SupplyType::Battery,
                None,
                Some("Charging"),
                Some(50),
            ),
        ];
        assert!(is_on_ac_power(&supplies));
    }

    #[test]
    fn test_mains_offline_battery_full() {
        let supplies = vec![
            make_supply("AC0", SupplyType::Mains, Some(false), None, None),
            make_supply("BAT0", SupplyType::Battery, None, Some("Full"), Some(100)),
        ];
        assert!(is_on_ac_power(&supplies));
    }

    #[test]
    fn test_battery_only_discharging() {
        let supplies = vec![make_supply(
            "BAT0",
            SupplyType::Battery,
            None,
            Some("Discharging"),
            Some(45),
        )];
        assert!(!is_on_ac_power(&supplies));
    }

    #[test]
    fn test_usb_supply_online() {
        let supplies = vec![make_supply("USB0", SupplyType::Usb, Some(true), None, None)];
        assert!(is_on_ac_power(&supplies));
    }

    #[test]
    fn test_ups_online() {
        let supplies = vec![make_supply("UPS0", SupplyType::Ups, Some(true), None, None)];
        assert!(is_on_ac_power(&supplies));
    }

    #[test]
    fn test_has_battery_true() {
        let supplies = vec![make_supply(
            "BAT0",
            SupplyType::Battery,
            None,
            Some("Full"),
            Some(100),
        )];
        assert!(has_battery(&supplies));
    }

    #[test]
    fn test_has_battery_false() {
        let supplies = vec![make_supply(
            "AC0",
            SupplyType::Mains,
            Some(true),
            None,
            None,
        )];
        assert!(!has_battery(&supplies));
    }

    #[test]
    fn test_min_battery_capacity() {
        let supplies = vec![
            make_supply(
                "BAT0",
                SupplyType::Battery,
                None,
                Some("Discharging"),
                Some(72),
            ),
            make_supply(
                "BAT1",
                SupplyType::Battery,
                None,
                Some("Discharging"),
                Some(30),
            ),
        ];
        assert_eq!(min_battery_capacity(&supplies), Some(30));
    }

    #[test]
    fn test_min_battery_capacity_none() {
        let supplies = vec![make_supply(
            "AC0",
            SupplyType::Mains,
            Some(true),
            None,
            None,
        )];
        assert_eq!(min_battery_capacity(&supplies), None);
    }

    #[test]
    fn test_supply_type_from_str() {
        assert_eq!(SupplyType::from_str("Mains"), SupplyType::Mains);
        assert_eq!(SupplyType::from_str("Battery"), SupplyType::Battery);
        assert_eq!(SupplyType::from_str("UPS"), SupplyType::Ups);
        assert_eq!(SupplyType::from_str("USB"), SupplyType::Usb);
        assert_eq!(SupplyType::from_str("mains\n"), SupplyType::Mains);
        assert_eq!(SupplyType::from_str("something"), SupplyType::Unknown);
    }

    #[test]
    fn test_enumerate_does_not_panic() {
        // Should work regardless of whether /sys/class/power_supply exists
        let _ = enumerate_power_supplies();
    }

    #[test]
    fn test_battery_not_charging_implies_ac() {
        // "Not charging" status means the battery is connected to AC but
        // the charge controller has decided not to charge (e.g. threshold
        // reached). This should be treated as on-AC.
        let supplies = vec![make_supply(
            "BAT0",
            SupplyType::Battery,
            None,
            Some("Not charging"),
            Some(80),
        )];
        assert!(is_on_ac_power(&supplies));
    }
}

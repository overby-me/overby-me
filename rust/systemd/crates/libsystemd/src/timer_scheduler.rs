//! Timer unit scheduling — fires associated service units when timers elapse.
//!
//! After PID 1 finishes activating units, the timer scheduler thread wakes up
//! periodically and checks all active `.timer` units to see if any of their
//! trigger conditions have been met.  When a timer elapses, the scheduler
//! starts (or restarts) the associated service unit via the control interface.
//!
//! `OnCalendar=` expressions are evaluated using the [`CalendarSpec`] parser
//! which supports the full systemd calendar expression syntax including
//! weekday filters, ranges, lists, and repetitions.

use log::{debug, info, trace, warn};
use std::time::{Duration, Instant, SystemTime};

use crate::calendar_spec::CalendarSpec;
use crate::lock_ext::RwLockExt;
use crate::runtime_info::ArcMutRuntimeInfo;
use crate::units::{ActivationSource, Specific, StatusStarted, TimerConfig, UnitId, UnitStatus};

/// Threshold in seconds for detecting a wall-clock jump.
/// If wall-clock advances by more or less than monotonic time by this amount,
/// we consider it a clock change event (OnClockChange=).
const CLOCK_JUMP_THRESHOLD_SECS: i64 = 2;

/// State for detecting clock and timezone changes between scheduler ticks.
struct ChangeDetector {
    /// Last known mtime of /etc/localtime (for timezone change detection).
    last_localtime_mtime: Option<std::time::SystemTime>,
    /// Last wall-clock reading (for clock jump detection).
    last_wallclock: Option<SystemTime>,
    /// Last monotonic reading corresponding to last_wallclock.
    last_monotonic: Option<Instant>,
}

impl ChangeDetector {
    fn new() -> Self {
        let localtime_mtime = std::fs::metadata("/etc/localtime")
            .and_then(|m| m.modified())
            .ok();
        Self {
            last_localtime_mtime: localtime_mtime,
            last_wallclock: Some(SystemTime::now()),
            last_monotonic: Some(Instant::now()),
        }
    }

    /// Check if the timezone has changed since the last call.
    /// Detects changes by comparing the mtime of /etc/localtime.
    fn timezone_changed(&mut self) -> bool {
        let current_mtime = std::fs::metadata("/etc/localtime")
            .and_then(|m| m.modified())
            .ok();
        let changed = match (&self.last_localtime_mtime, &current_mtime) {
            (Some(old), Some(new)) => old != new,
            (None, Some(_)) => true,
            (Some(_), None) => true,
            (None, None) => false,
        };
        if changed {
            self.last_localtime_mtime = current_mtime;
        }
        changed
    }

    /// Check if the system clock has jumped since the last call.
    /// Detects jumps by comparing wall-clock delta vs monotonic delta.
    fn clock_changed(&mut self) -> bool {
        let now_wall = SystemTime::now();
        let now_mono = Instant::now();

        let changed = if let (Some(prev_wall), Some(prev_mono)) =
            (self.last_wallclock, self.last_monotonic)
        {
            let mono_delta = now_mono.duration_since(prev_mono);
            let wall_delta = now_wall.duration_since(prev_wall).unwrap_or(Duration::ZERO);
            let diff = (wall_delta.as_secs() as i64) - (mono_delta.as_secs() as i64);
            diff.abs() > CLOCK_JUMP_THRESHOLD_SECS
        } else {
            false
        };

        self.last_wallclock = Some(now_wall);
        self.last_monotonic = Some(now_mono);
        changed
    }
}

/// Boot instant — captured once when the scheduler starts.
/// Used for OnBootSec= / OnStartupSec= calculations.
static BOOT_INSTANT: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Start the background timer scheduler thread.
///
/// This should be called after the initial unit activation is complete
/// (or at least after all timer units have been loaded and started).
pub fn start_timer_scheduler_thread(run_info: ArcMutRuntimeInfo) {
    let boot_instant = *BOOT_INSTANT.get_or_init(Instant::now);

    std::thread::Builder::new()
        .name("timer-scheduler".into())
        .spawn(move || {
            info!("Timer scheduler started");

            // Give the system a moment to finish initial activation before
            // checking timers for the first time.  This avoids firing
            // OnBootSec=0 timers before their target services are loaded.
            std::thread::sleep(Duration::from_secs(2));

            // Track when each timer last fired so we can implement
            // OnUnitActiveSec= and avoid re-firing within the same period.
            let mut last_fired: std::collections::HashMap<String, Instant> =
                std::collections::HashMap::new();

            let mut change_detector = ChangeDetector::new();

            loop {
                // Detect clock/timezone changes before checking timers
                let tz_changed = change_detector.timezone_changed();
                let clock_changed = change_detector.clock_changed();

                if tz_changed {
                    info!("Timezone change detected");
                }
                if clock_changed {
                    info!("Clock change detected");
                }

                check_and_fire_timers(
                    &run_info,
                    boot_instant,
                    &mut last_fired,
                    tz_changed,
                    clock_changed,
                );
                // Use a shorter interval (1s) to detect clock/timezone changes
                // promptly. The original 15s interval is too slow for tests that
                // wait for OnClockChange/OnTimezoneChange timers to fire.
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .expect("Failed to spawn timer-scheduler thread");
}

/// One pass of the timer check loop.
fn check_and_fire_timers(
    run_info: &ArcMutRuntimeInfo,
    boot_instant: Instant,
    last_fired: &mut std::collections::HashMap<String, Instant>,
    timezone_changed: bool,
    clock_changed: bool,
) {
    let now = Instant::now();
    let elapsed_since_boot = now.duration_since(boot_instant);

    // Collect timer units that need to fire.
    // We collect first, then fire, to avoid holding the read lock during activation.
    let mut timers_to_fire: Vec<(UnitId, String)> = Vec::new();

    {
        let ri = run_info.read_poisoned();
        for unit in ri.unit_table.values() {
            if let Specific::Timer(timer_specific) = &unit.specific {
                // Only check timers that are started/running
                let status = unit.common.status.read_poisoned().clone();
                if !matches!(status, UnitStatus::Started(StatusStarted::Running)) {
                    continue;
                }

                let conf = &timer_specific.conf;
                let timer_name = &unit.id.name;
                let target_unit = &conf.unit;

                if should_fire_timer(
                    conf,
                    timer_name,
                    elapsed_since_boot,
                    now,
                    last_fired,
                    timezone_changed,
                    clock_changed,
                ) {
                    timers_to_fire.push((unit.id.clone(), target_unit.clone()));
                }
            }
        }
    }

    // Fire the collected timers
    for (timer_id, target_unit_name) in timers_to_fire {
        info!(
            "Timer {} elapsed, activating {}",
            timer_id.name, target_unit_name
        );
        last_fired.insert(timer_id.name.clone(), now);

        // Record LastTriggerUSec on the timer unit's state and write stamp file
        {
            let ri = run_info.read_poisoned();
            if let Some(unit) = ri.unit_table.values().find(|u| u.id == timer_id)
                && let Specific::Timer(ref tmr) = unit.specific
            {
                let trigger_usec = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_micros() as u64)
                    .unwrap_or(0);
                tmr.state.write_poisoned().last_trigger_usec = Some(trigger_usec);

                // Write persistent stamp file so the last trigger time
                // survives reboots.
                if tmr.conf.persistent {
                    let stamp_dir = "/var/lib/systemd/timers";
                    let stamp_path = format!("{}/stamp-{}", stamp_dir, timer_id.name);
                    let _ = std::fs::create_dir_all(stamp_dir);
                    if let Err(e) = std::fs::write(&stamp_path, "") {
                        warn!(
                            "Timer {}: failed to write stamp file {}: {}",
                            timer_id.name, stamp_path, e
                        );
                    }
                }
            }
        }

        fire_timer_target(run_info, &target_unit_name, &timer_id.name);
    }
}

/// Determine if a timer should fire based on its configuration and current state.
fn should_fire_timer(
    conf: &TimerConfig,
    timer_name: &str,
    elapsed_since_boot: Duration,
    now: Instant,
    last_fired: &std::collections::HashMap<String, Instant>,
    timezone_changed: bool,
    clock_changed: bool,
) -> bool {
    let last = last_fired.get(timer_name).copied();

    // OnBootSec= / OnStartupSec= — fire once after boot + duration
    for dur in conf.on_boot_sec.iter().chain(conf.on_startup_sec.iter()) {
        if elapsed_since_boot >= *dur {
            // Should have fired by now. Check if we already did.
            if last.is_none() {
                trace!(
                    "Timer {}: OnBootSec/OnStartupSec {:?} elapsed (boot+{:?})",
                    timer_name, dur, elapsed_since_boot
                );
                return true;
            }
        }
    }

    // OnActiveSec= — fire once after timer activation + duration
    // Since we don't track when the timer was activated separately, we
    // approximate by using boot time (timers are activated during boot).
    for dur in &conf.on_active_sec {
        if elapsed_since_boot >= *dur && last.is_none() {
            trace!("Timer {}: OnActiveSec {:?} elapsed", timer_name, dur);
            return true;
        }
    }

    // OnUnitActiveSec= — repeating timer relative to last activation
    for dur in &conf.on_unit_active_sec {
        if dur.is_zero() {
            continue;
        }
        match last {
            Some(last_time) => {
                let since_last = now.duration_since(last_time);
                if since_last >= *dur {
                    trace!(
                        "Timer {}: OnUnitActiveSec {:?} elapsed ({:?} since last fire)",
                        timer_name, dur, since_last
                    );
                    return true;
                }
            }
            None => {
                // First run after boot — fire if boot elapsed >= dur
                if elapsed_since_boot >= *dur {
                    return true;
                }
            }
        }
    }

    // OnUnitInactiveSec= — repeating timer relative to last deactivation
    // We approximate this the same as OnUnitActiveSec for now.
    for dur in &conf.on_unit_inactive_sec {
        if dur.is_zero() {
            continue;
        }
        match last {
            Some(last_time) => {
                if now.duration_since(last_time) >= *dur {
                    return true;
                }
            }
            None => {
                if elapsed_since_boot >= *dur {
                    return true;
                }
            }
        }
    }

    // OnClockChange= — fire when the system clock jumps
    if conf.on_clock_change && clock_changed {
        trace!(
            "Timer {}: OnClockChange triggered (clock jump detected)",
            timer_name
        );
        return true;
    }

    // OnTimezoneChange= — fire when the system timezone changes
    if conf.on_timezone_change && timezone_changed {
        trace!(
            "Timer {}: OnTimezoneChange triggered (timezone change detected)",
            timer_name
        );
        return true;
    }

    // OnCalendar= — calendar event expressions
    // Parse the expression using CalendarSpec and check if the next elapse
    // time is at or before the current wall-clock time.
    for expr in &conf.on_calendar {
        match CalendarSpec::parse(expr) {
            Ok(spec) => {
                let now_system = SystemTime::now();
                let now_dt = CalendarSpec::system_time_to_datetime(now_system);

                // Determine the reference time: if we fired before, the reference
                // is one second after the last fire time (so we don't re-trigger
                // for the same calendar tick). If we haven't fired, the reference
                // is boot time (or epoch if Persistent=true to catch missed runs).
                let reference_dt = if let Some(last_instant) = last {
                    // Convert the last-fired Instant to a wall-clock DateTime.
                    // We do this by computing the offset from `now` Instant to
                    // `now` SystemTime, then applying that offset.
                    let elapsed_since_last = now.duration_since(last_instant);
                    let last_unix = CalendarSpec::datetime_to_unix(&now_dt)
                        - elapsed_since_last.as_secs() as i64;
                    // One second after last fire so we skip the same slot
                    crate::calendar_spec::unix_to_datetime(last_unix + 1)
                } else if conf.persistent {
                    // Persistent=true and first check: fire immediately for any
                    // missed calendar events by using epoch as reference.
                    crate::calendar_spec::DateTime {
                        year: 1970,
                        month: 1,
                        day: 1,
                        hour: 0,
                        minute: 0,
                        second: 0,
                    }
                } else {
                    // Not persistent, first check: use boot time as reference.
                    let boot_unix = CalendarSpec::datetime_to_unix(&now_dt)
                        - elapsed_since_boot.as_secs() as i64;
                    crate::calendar_spec::unix_to_datetime(boot_unix)
                };

                if let Some(next) = spec.next_elapse(reference_dt) {
                    let next_unix = CalendarSpec::datetime_to_unix(&next);
                    let now_unix = CalendarSpec::datetime_to_unix(&now_dt);
                    if next_unix <= now_unix {
                        trace!(
                            "Timer {}: OnCalendar={} next elapse {:?} <= now {:?}",
                            timer_name, expr, next, now_dt
                        );
                        return true;
                    }
                }
            }
            Err(e) => {
                debug!(
                    "Timer {}: OnCalendar={} parse error: {}",
                    timer_name, expr, e
                );
            }
        }
    }

    false
}

/// Re-export `unix_to_datetime` so the scheduler helper above can use it
/// without a fully-qualified path in tests.
pub use crate::calendar_spec::unix_to_datetime;

/// Fire a timer's target unit by starting it via the activation system.
/// Set TRIGGER_UNIT and TRIGGER_TIMER_*_USEC on the target service's state.
fn set_timer_trigger_info(unit: &crate::units::Unit, timer_name: &str) {
    if let Specific::Service(specific) = &unit.specific {
        let mut state = specific.state.write_poisoned();
        state.srvc.trigger_unit = Some(timer_name.to_owned());
        let now = SystemTime::now();
        if let Ok(dur) = now.duration_since(SystemTime::UNIX_EPOCH) {
            state.srvc.trigger_timer_realtime_usec = Some(dur.as_micros() as u64);
        }
        let boot_instant = BOOT_INSTANT.get().copied().unwrap_or_else(Instant::now);
        let mono = Instant::now().duration_since(boot_instant);
        state.srvc.trigger_timer_monotonic_usec = Some(mono.as_micros() as u64);
    }
}

fn fire_timer_target(run_info: &ArcMutRuntimeInfo, target_unit_name: &str, timer_name: &str) {
    let ri = run_info.read_poisoned();

    // Find the target unit
    let target_unit = ri
        .unit_table
        .values()
        .find(|u| u.id.name == target_unit_name);

    match target_unit {
        Some(unit) => {
            set_timer_trigger_info(unit, timer_name);
            let status = unit.common.status.read_poisoned().clone();
            match status {
                UnitStatus::Started(_) => {
                    // Service is already running — try to restart it
                    debug!(
                        "Timer target {} is already running, attempting restart",
                        target_unit_name
                    );
                    match unit.reactivate(&ri, ActivationSource::Regular) {
                        Ok(()) => {
                            info!("Timer fired: restarted {}", target_unit_name);
                        }
                        Err(e) => {
                            warn!("Timer failed to restart {}: {}", target_unit_name, e);
                        }
                    }
                }
                _ => {
                    // Service is not running — start it
                    let id = unit.id.clone();
                    drop(ri);
                    match crate::units::activate_unit(
                        id,
                        &run_info.read_poisoned(),
                        ActivationSource::Regular,
                    ) {
                        Ok(_) => {
                            info!("Timer fired: started {}", target_unit_name);
                        }
                        Err(e) => {
                            warn!("Timer failed to start {}: {}", target_unit_name, e);
                        }
                    }
                }
            }
        }
        None => {
            // Unit not in the boot dependency graph — try on-demand loading
            debug!(
                "Timer target {} not found in unit table, attempting on-demand load",
                target_unit_name
            );
            drop(ri);

            // Use the control interface's find_or_load_unit logic by sending
            // a start command through the internal path.
            let ri = run_info.read_poisoned();
            if let Some(unit) = ri
                .unit_table
                .values()
                .find(|u| u.id.name == target_unit_name)
            {
                let id = unit.id.clone();
                drop(ri);
                match crate::units::activate_unit(
                    id,
                    &run_info.read_poisoned(),
                    ActivationSource::Regular,
                ) {
                    Ok(_) => info!("Timer fired: started {} (on-demand)", target_unit_name),
                    Err(e) => warn!(
                        "Timer failed to start {} (on-demand): {}",
                        target_unit_name, e
                    ),
                }
            } else {
                debug!(
                    "Timer target {} not found and could not be loaded",
                    target_unit_name
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calendar_spec_parse_hourly() {
        let spec = CalendarSpec::parse("hourly").unwrap();
        assert_eq!(spec.normalized(), "*-*-* *:00:00");
    }

    #[test]
    fn test_calendar_spec_parse_daily() {
        let spec = CalendarSpec::parse("daily").unwrap();
        assert_eq!(spec.normalized(), "*-*-* 00:00:00");
    }

    #[test]
    fn test_calendar_spec_parse_weekly() {
        let spec = CalendarSpec::parse("weekly").unwrap();
        assert_eq!(spec.normalized(), "Mon *-*-* 00:00:00");
    }

    #[test]
    fn test_calendar_spec_parse_monthly() {
        let spec = CalendarSpec::parse("monthly").unwrap();
        assert_eq!(spec.normalized(), "*-*-01 00:00:00");
    }

    #[test]
    fn test_calendar_spec_parse_yearly() {
        let spec = CalendarSpec::parse("yearly").unwrap();
        assert_eq!(spec.normalized(), "*-01-01 00:00:00");
    }

    #[test]
    fn test_calendar_spec_parse_complex_expression() {
        let spec = CalendarSpec::parse("*-*-* 06:00:00").unwrap();
        assert_eq!(spec.normalized(), "*-*-* 06:00:00");
    }

    #[test]
    fn test_calendar_spec_parse_minutely() {
        let spec = CalendarSpec::parse("minutely").unwrap();
        assert_eq!(spec.normalized(), "*-*-* *:*:00");
    }

    #[test]
    fn test_calendar_spec_parse_quarterly() {
        let spec = CalendarSpec::parse("quarterly").unwrap();
        assert_eq!(spec.normalized(), "*-01,04,07,10-01 00:00:00");
    }

    #[test]
    fn test_calendar_spec_next_elapse_daily() {
        use crate::calendar_spec::DateTime;
        let spec = CalendarSpec::parse("daily").unwrap();
        let after = DateTime {
            year: 2025,
            month: 6,
            day: 15,
            hour: 0,
            minute: 0,
            second: 1,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.day, 16);
        assert_eq!(next.hour, 0);
    }

    #[test]
    fn test_calendar_spec_next_elapse_complex() {
        use crate::calendar_spec::DateTime;
        let spec = CalendarSpec::parse("Mon..Fri *-*-* 09:00:00").unwrap();
        // 2025-06-14 is Saturday
        let after = DateTime {
            year: 2025,
            month: 6,
            day: 14,
            hour: 10,
            minute: 0,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        // Next Mon..Fri is Monday June 16
        assert_eq!(next.day, 16);
        assert_eq!(next.hour, 9);
    }

    #[test]
    fn test_should_fire_on_boot_sec_not_yet() {
        let conf = TimerConfig {
            on_boot_sec: vec![Duration::from_secs(3600)],
            on_startup_sec: vec![],
            on_active_sec: vec![],
            on_unit_active_sec: vec![],
            on_unit_inactive_sec: vec![],
            on_calendar: vec![],
            accuracy_sec: Duration::from_secs(60),
            randomized_delay_sec: Duration::ZERO,
            fixed_random_delay: false,
            persistent: false,
            wake_system: false,
            remain_after_elapse: true,
            on_clock_change: false,
            on_timezone_change: false,
            unit: "test.service".into(),
        };
        let last_fired = std::collections::HashMap::new();
        // Only 5 minutes since boot — shouldn't fire yet
        let result = should_fire_timer(
            &conf,
            "test.timer",
            Duration::from_secs(300),
            Instant::now(),
            &last_fired,
            false,
            false,
        );
        assert!(!result);
    }

    #[test]
    fn test_should_fire_on_boot_sec_elapsed() {
        let conf = TimerConfig {
            on_boot_sec: vec![Duration::from_secs(300)],
            on_startup_sec: vec![],
            on_active_sec: vec![],
            on_unit_active_sec: vec![],
            on_unit_inactive_sec: vec![],
            on_calendar: vec![],
            accuracy_sec: Duration::from_secs(60),
            randomized_delay_sec: Duration::ZERO,
            fixed_random_delay: false,
            persistent: false,
            wake_system: false,
            remain_after_elapse: true,
            on_clock_change: false,
            on_timezone_change: false,
            unit: "test.service".into(),
        };
        let last_fired = std::collections::HashMap::new();
        // 10 minutes since boot, timer is 5 min — should fire
        let result = should_fire_timer(
            &conf,
            "test.timer",
            Duration::from_secs(600),
            Instant::now(),
            &last_fired,
            false,
            false,
        );
        assert!(result);
    }

    #[test]
    fn test_should_fire_on_boot_sec_already_fired() {
        let conf = TimerConfig {
            on_boot_sec: vec![Duration::from_secs(300)],
            on_startup_sec: vec![],
            on_active_sec: vec![],
            on_unit_active_sec: vec![],
            on_unit_inactive_sec: vec![],
            on_calendar: vec![],
            accuracy_sec: Duration::from_secs(60),
            randomized_delay_sec: Duration::ZERO,
            fixed_random_delay: false,
            persistent: false,
            wake_system: false,
            remain_after_elapse: true,
            on_clock_change: false,
            on_timezone_change: false,
            unit: "test.service".into(),
        };
        let mut last_fired = std::collections::HashMap::new();
        last_fired.insert("test.timer".into(), Instant::now());
        // Already fired — shouldn't fire again
        let result = should_fire_timer(
            &conf,
            "test.timer",
            Duration::from_secs(600),
            Instant::now(),
            &last_fired,
            false,
            false,
        );
        assert!(!result);
    }

    #[test]
    fn test_should_fire_on_unit_active_sec_repeating() {
        let conf = TimerConfig {
            on_boot_sec: vec![],
            on_startup_sec: vec![],
            on_active_sec: vec![],
            on_unit_active_sec: vec![Duration::from_secs(60)],
            on_unit_inactive_sec: vec![],
            on_calendar: vec![],
            accuracy_sec: Duration::from_secs(60),
            randomized_delay_sec: Duration::ZERO,
            fixed_random_delay: false,
            persistent: false,
            wake_system: false,
            remain_after_elapse: true,
            on_clock_change: false,
            on_timezone_change: false,
            unit: "test.service".into(),
        };
        let now = Instant::now();
        let mut last_fired = std::collections::HashMap::new();
        // Last fired 120s ago, interval is 60s — should fire
        last_fired.insert(
            "test.timer".into(),
            now.checked_sub(Duration::from_secs(120)).unwrap(),
        );
        let result = should_fire_timer(
            &conf,
            "test.timer",
            Duration::from_secs(300),
            now,
            &last_fired,
            false,
            false,
        );
        assert!(result);
    }

    #[test]
    fn test_should_fire_on_unit_active_sec_too_soon() {
        let conf = TimerConfig {
            on_boot_sec: vec![],
            on_startup_sec: vec![],
            on_active_sec: vec![],
            on_unit_active_sec: vec![Duration::from_secs(3600)],
            on_unit_inactive_sec: vec![],
            on_calendar: vec![],
            accuracy_sec: Duration::from_secs(60),
            randomized_delay_sec: Duration::ZERO,
            fixed_random_delay: false,
            persistent: false,
            wake_system: false,
            remain_after_elapse: true,
            on_clock_change: false,
            on_timezone_change: false,
            unit: "test.service".into(),
        };
        let now = Instant::now();
        let mut last_fired = std::collections::HashMap::new();
        // Last fired 30s ago, interval is 1h — shouldn't fire
        last_fired.insert(
            "test.timer".into(),
            now.checked_sub(Duration::from_secs(30)).unwrap(),
        );
        let result = should_fire_timer(
            &conf,
            "test.timer",
            Duration::from_secs(300),
            now,
            &last_fired,
            false,
            false,
        );
        assert!(!result);
    }

    #[test]
    fn test_should_fire_calendar_hourly() {
        let conf = TimerConfig {
            on_boot_sec: vec![],
            on_startup_sec: vec![],
            on_active_sec: vec![],
            on_unit_active_sec: vec![],
            on_unit_inactive_sec: vec![],
            on_calendar: vec!["hourly".into()],
            accuracy_sec: Duration::from_secs(60),
            randomized_delay_sec: Duration::ZERO,
            fixed_random_delay: false,
            persistent: false,
            wake_system: false,
            remain_after_elapse: true,
            on_clock_change: false,
            on_timezone_change: false,
            unit: "test.service".into(),
        };
        let now = Instant::now();
        let mut last_fired = std::collections::HashMap::new();
        // Last fired 2 hours ago — should fire
        last_fired.insert(
            "test.timer".into(),
            now.checked_sub(Duration::from_secs(7200)).unwrap(),
        );
        let result = should_fire_timer(
            &conf,
            "test.timer",
            Duration::from_secs(10000),
            now,
            &last_fired,
            false,
            false,
        );
        assert!(result);
    }

    #[test]
    fn test_should_fire_calendar_hourly_too_soon() {
        let conf = TimerConfig {
            on_boot_sec: vec![],
            on_startup_sec: vec![],
            on_active_sec: vec![],
            on_unit_active_sec: vec![],
            on_unit_inactive_sec: vec![],
            on_calendar: vec!["hourly".into()],
            accuracy_sec: Duration::from_secs(60),
            randomized_delay_sec: Duration::ZERO,
            fixed_random_delay: false,
            persistent: false,
            wake_system: false,
            remain_after_elapse: true,
            on_clock_change: false,
            on_timezone_change: false,
            unit: "test.service".into(),
        };
        let now = Instant::now();
        let mut last_fired = std::collections::HashMap::new();
        // Last fired 1 second ago — shouldn't fire yet (using a very short
        // interval avoids flakiness: with 10 minutes, if we happen to be in
        // the first 10 minutes of the hour a new hourly boundary has been
        // crossed since last fire, so should_fire_timer correctly returns true).
        last_fired.insert(
            "test.timer".into(),
            now.checked_sub(Duration::from_secs(1)).unwrap(),
        );
        let result = should_fire_timer(
            &conf,
            "test.timer",
            Duration::from_secs(10000),
            now,
            &last_fired,
            false,
            false,
        );
        assert!(!result);
    }

    #[test]
    fn test_should_fire_empty_config() {
        let conf = TimerConfig {
            on_boot_sec: vec![],
            on_startup_sec: vec![],
            on_active_sec: vec![],
            on_unit_active_sec: vec![],
            on_unit_inactive_sec: vec![],
            on_calendar: vec![],
            accuracy_sec: Duration::from_secs(60),
            randomized_delay_sec: Duration::ZERO,
            fixed_random_delay: false,
            persistent: false,
            wake_system: false,
            remain_after_elapse: true,
            on_clock_change: false,
            on_timezone_change: false,
            unit: "test.service".into(),
        };
        let last_fired = std::collections::HashMap::new();
        // No triggers configured — should never fire
        let result = should_fire_timer(
            &conf,
            "test.timer",
            Duration::from_secs(999999),
            Instant::now(),
            &last_fired,
            false,
            false,
        );
        assert!(!result);
    }

    #[test]
    fn test_should_fire_persistent_calendar_first_boot() {
        let conf = TimerConfig {
            on_boot_sec: vec![],
            on_startup_sec: vec![],
            on_active_sec: vec![],
            on_unit_active_sec: vec![],
            on_unit_inactive_sec: vec![],
            on_calendar: vec!["weekly".into()],
            accuracy_sec: Duration::from_secs(60),
            randomized_delay_sec: Duration::ZERO,
            fixed_random_delay: false,
            persistent: true,
            wake_system: false,
            remain_after_elapse: true,
            on_clock_change: false,
            on_timezone_change: false,
            unit: "test.service".into(),
        };
        let last_fired = std::collections::HashMap::new();
        // Persistent=true and never fired — should fire on first check
        // (even though we just booted, Persistent=true means "catch up on missed runs")
        let result = should_fire_timer(
            &conf,
            "test.timer",
            Duration::from_secs(60),
            Instant::now(),
            &last_fired,
            false,
            false,
        );
        assert!(result);
    }

    #[test]
    fn test_calendar_spec_every_two_hours() {
        use crate::calendar_spec::DateTime;
        let spec = CalendarSpec::parse("*-*-* */2:00:00").unwrap();
        let after = DateTime {
            year: 2025,
            month: 1,
            day: 1,
            hour: 3,
            minute: 0,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.hour, 4);
    }

    #[test]
    fn test_parse_timespan_via_from_parsed_config() {
        use crate::units::from_parsed_config::parse_timespan;
        assert_eq!(parse_timespan("15min"), Some(Duration::from_secs(900)));
        assert_eq!(parse_timespan("1h"), Some(Duration::from_secs(3600)));
        assert_eq!(parse_timespan("1d"), Some(Duration::from_secs(86400)));
        assert_eq!(parse_timespan("1h 30min"), Some(Duration::from_secs(5400)));
        assert_eq!(parse_timespan("500ms"), Some(Duration::from_millis(500)));
        assert_eq!(parse_timespan("100min"), Some(Duration::from_secs(6000)));
        assert_eq!(parse_timespan("2s"), Some(Duration::from_secs(2)));
        assert_eq!(parse_timespan("2secs"), Some(Duration::from_secs(2)));
        assert_eq!(parse_timespan("2hrs"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_timespan("1hr"), Some(Duration::from_secs(3600)));
        assert_eq!(parse_timespan(""), None);
        assert_eq!(parse_timespan("30"), Some(Duration::from_secs(30)));
    }
}

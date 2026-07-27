{
  name = "30-ONCLOCKCHANGE";
  # Re-baselined 2026-07-27. The previous rationale said the alternate-path
  # section "requires D-Bus integration between timedated and PID 1 for
  # cross-process timezone change notification, which is not yet implemented",
  # and truncated the script with `touch /skipped; exit 0`.
  #
  # That looks stale on both sides, which is why the override is removed here
  # rather than reworded:
  #   - crates/timedated honours SYSTEMD_ETC_LOCALTIME and SYSTEMD_ETC_ADJTIME
  #     already (main.rs localtime_path()/adjtime_path()).
  #   - PID 1 already applies ManagerEnvironment= to its own environment, and
  #     timer_scheduler.rs's ChangeDetector tracks a VECTOR of localtime paths,
  #     "always /etc/localtime, plus any SYSTEMD_ETC_LOCALTIME override".
  # Upstream does not use D-Bus for this either: manager_setup_timezone_change()
  # watches etc_localtime() with inotify.
  #
  # Running it unmasked is the cheapest way to find out what actually fails,
  # if anything. The first section (--on-timezone-change / --on-clock-change
  # transient timers) already passed under the old truncated script.
}

{
  name = "53-TIMER";
  # Skip timer subtests that depend on time-jump detection:
  # - RandomizedDelaySec-reload: recalculates to next occurrence instead
  #   of staying within the original window after a time jump.
  # - restart-trigger: timer doesn't fire when system clock jumps past
  #   OnCalendar= time.
  patchScript = ''
    rm -f TEST-53-TIMER.RandomizedDelaySec-reload.sh
    rm -f TEST-53-TIMER.restart-trigger.sh
  '';
}

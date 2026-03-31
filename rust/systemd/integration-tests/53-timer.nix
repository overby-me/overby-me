{
  name = "53-TIMER";
  # Skip RandomizedDelaySec-reload subtest: recalculates to next occurrence
  # instead of staying within the original window after a time jump.
  # restart-trigger is now enabled — clock-jump detection uses pre-jump
  # wall-clock time as reference for OnCalendar= re-evaluation.
  patchScript = ''
    rm -f TEST-53-TIMER.RandomizedDelaySec-reload.sh
  '';
}

{
  name = "17-UDEV";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.failed-event\\.sh$";
  };
  # FAILING, and deliberately left that way: it carries no override, so the red
  # result is honest. Diagnosed 2026-07-27 while validating two unrelated udev
  # changes; this failure predates them and is not a regression.
  #
  # The subtest has two halves and rust-systemd satisfies neither.
  #
  # run_test_timeout writes `event_timeout=10` into /run/udev/udev.conf.d/ and a
  # rule whose PROGRAM runs `sleep 60`. udevd must kill that program when the
  # event timeout expires, treat the PROGRAM as not matching (so the following
  # `PROGRAM!=` fires and sets ENV{PROGRAM_RESULT}=KILLED), and finish the event
  # normally without the worker itself being reported as failed.
  # run_program_capture in crates/udevd/src/lib.rs spawns with no timeout at all
  # and waits forever, and event_timeout is parsed only to be discarded:
  #     let _event_timeout = args.event_timeout;
  # PROGRAM_RESULT appears nowhere in the tree.
  #
  # run_test_killed sends SIGABRT to a process named `udev-worker` and expects
  # UDEV_WORKER_FAILED=1, UDEV_WORKER_SIGNAL=6 and UDEV_WORKER_SIGNAL_NAME=ABRT
  # on the monitor. rust-systemd runs workers as THREADS, named
  # `udev-worker:<devpath>`, so there is no process for pkill to signal and no
  # notion of a worker dying by signal to report. This half needs a worker
  # process model, not a small fix.
  #
  # The spawn deadline is now implemented: udev.conf and its drop-ins are read
  # for event_timeout=, and a PROGRAM= that outlives it is killed and counts as
  # a non-match instead of blocking forever. That was worth doing on its own,
  # because a hanging rule used to wedge its event indefinitely.
  #
  # It did NOT move this test, so the timeout half is NOT simply the missing
  # deadline. Measured after the change: `udevadm trigger --action add /dev/null`
  # reports "triggered 1 device(s)" and `udevadm monitor --udev --property
  # --subsystem-match=mem` is running, but PROGRAM_RESULT never appears in its
  # output. Note ENV{PROGRAM_RESULT} is set by the RULE, not by udevd, so the
  # remaining question is which of these is true, and it has not been measured:
  #   - the rule never fires, i.e. `PROGRAM!="/usr/bin/sleep 60"` does not
  #     evaluate as a match when the program fails or is killed. Worth checking
  #     that /usr/bin/sleep does not exist on NixOS at all, which should make
  #     the != match outright, independently of any timeout;
  #   - or the rule does fire and `udevadm monitor --property` does not emit the
  #     device's properties, in which case the gap is in monitor, not in rules.
  # `udevadm info /dev/null` after the trigger separates the two: if
  # PROGRAM_RESULT is in the database, the rule fired and monitor is at fault.
}

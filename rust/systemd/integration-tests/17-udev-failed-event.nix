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
  # The timeout half is the bounded one and is worth doing on its own: honour
  # event_timeout= and timeout_signal= from udev.conf.d, give a spawned PROGRAM=
  # a deadline, and kill and report rather than block forever. A rule that hangs
  # currently wedges its event indefinitely, which matters well beyond this test.
}

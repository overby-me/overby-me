{
  name = "17-UDEV";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.failed-event\\.sh$";
  };
  # FAILING, and deliberately left that way: it carries no override, so the red
  # result is honest. The subtest has two halves; the first now passes.
  #
  # run_test_timeout PASSES. It writes `event_timeout=10` into
  # /run/udev/udev.conf.d/ and a rule whose PROGRAM runs `sleep 60`. udevd kills
  # that program when the deadline expires, the following `PROGRAM!=` fires,
  # ENV{PROGRAM_RESULT}=KILLED reaches the monitor, and the worker is not
  # reported as failed.
  #
  # Two fixes got it there. udev.conf and its drop-ins were never parsed at all,
  # so event_timeout= was accepted on the command line only to be discarded;
  # they are read now, with /usr/lib, /run and /etc layered in that order and
  # refreshed on every reload. Then the spawn deadline that fix added turned out
  # to reach only IMPORT{program}=, because match_program() has its OWN spawn
  # path and never calls run_program_capture, so PROGRAM= still had no deadline
  # and `PROGRAM!="/usr/bin/sleep 60"` blocked its worker for the full sixty
  # seconds. Both paths now wait against the deadline on a helper thread that
  # drains the child's stdout, and kill on expiry.
  #
  # run_test_killed FAILS, and needs an architectural change. It sends SIGABRT
  # to a process named `udev-worker` and expects UDEV_WORKER_FAILED=1,
  # UDEV_WORKER_SIGNAL=6 and UDEV_WORKER_SIGNAL_NAME=ABRT on the monitor.
  # oxidized-systemd runs workers as THREADS named `udev-worker:<devpath>`, so there
  # is nothing for pkill to signal and no notion of a worker dying by signal to
  # report. That is a worker-process model, not a small fix.
  #
  # TWO WRONG TURNS ON THE WAY, recorded so they are not repeated:
  #   - the `udevadm monitor --property` theory. Dumping the udev database right
  #     after the trigger showed no PROGRAM_RESULT at all, and that property is
  #     set by the RULE rather than by udevd, so it never existed to be emitted.
  #   - a diagnostic placed in run_program_capture, whose silence was read as
  #     "PROGRAM rules never execute". The probe was simply in a function
  #     PROGRAM= does not use. When two functions do the same job, fixing one and
  #     instrumenting the other is how you get a confident wrong answer.
  #
  # Also verified by reading, so do not re-walk it: the reload path is sound.
  # `udevadm control --reload` sends RELOAD over the control socket, udevd's
  # handle_control_command sets *rules_reload_needed, and handle_client is
  # called from the MAIN LOOP with the same local the reload branch tests before
  # calling refresh_udev_config().
}

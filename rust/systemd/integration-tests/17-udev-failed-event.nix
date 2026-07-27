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
  # THE TIMEOUT HALF NOW PASSES. run_test_timeout completes and the script goes
  # on to run_test_killed, and PROGRAM_RESULT=KILLED appears on the monitor as
  # upstream expects.
  #
  # It took two fixes and one wrong turn. First, udev.conf and its drop-ins were
  # never parsed at all, so event_timeout= was accepted on the command line only
  # to be discarded; they are read now, with /usr/lib, /run and /etc layered in
  # that order and refreshed on every reload. Second, and this is the part that
  # actually mattered here, the spawn deadline that fix added reached only
  # IMPORT{program}=: match_program() has its OWN spawn path and never calls
  # run_program_capture, so PROGRAM= still had no deadline and
  # `PROGRAM!="/usr/bin/sleep 60"` blocked its worker for the full sixty
  # seconds. Both paths now wait against the deadline on a helper thread that
  # drains the child's stdout, and kill on expiry.
  #
  # THE WRONG TURN, worth remembering: a diagnostic was put in
  # run_program_capture and its silence was read as "PROGRAM rules never
  # execute". The probe was simply in a function PROGRAM= does not use. When two
  # functions do the same job, fixing one and instrumenting one is how you get a
  # confident wrong answer.
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
  # It did NOT move this test, and the reason is now MEASURED rather than
  # guessed. Dumping the udev database right after the trigger shows:
  #     E: MAJOR=1  DEVPATH=...  DEVNAME=null  MINOR=3  DEVMODE=0666  SUBSYSTEM=mem
  # and NO PROGRAM_RESULT. Since ENV{PROGRAM_RESULT} is set by the RULE, not by
  # udevd, that rules out the `udevadm monitor --property` theory: the property
  # never exists in the first place, so the gap is in RULE EVALUATION.
  #
  # The same run also reported `udevadm settle: timeout reached` after 15s, so
  # the event was still in flight. The test writes event_timeout=10 into
  # /run/udev/udev.conf.d/ and then runs `systemctl reload systemd-udevd.service`,
  # so the deadline looks like it is not being applied.
  #
  # THE RELOAD PATH IS NOT THE PROBLEM, verified by reading rather than
  # guessing: `udevadm control --reload` sends RELOAD over the control socket
  # (udevadm/src/main.rs), udevd's handle_control_command sets
  # *rules_reload_needed, and handle_client is called from the MAIN LOOP with
  # `&mut rules_reload_needed`, the same local the reload branch tests before
  # calling refresh_udev_config(). Socket, flag and consumer are all the same
  # object, so a reload does re-read udev.conf.d.
  #
  # NEXT MEASUREMENT: kmsg the EVENT_TIMEOUT value at the point
  # run_program_capture spawns, and see whether the worker actually sees 10 or
  # still 180. That distinguishes "the config was never re-read" from "it was
  # re-read but the deadline is not reaching the spawn", and it is one line.
  #
  # So the timeout half is NOT simply the missing deadline. Measured after the change: `udevadm trigger --action add /dev/null`
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

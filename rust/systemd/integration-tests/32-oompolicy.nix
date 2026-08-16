{
  # THIS TEST PASSES WITHOUT TESTING ANYTHING. Recorded 2026-07-28 so the green
  # is not mistaken for coverage.
  #
  # Upstream's TEST-32-OOMPOLICY.sh wraps its entire body in
  #
  #     if test -f /sys/fs/cgroup/system.slice/TEST-32-OOMPOLICY.service/memory.oom.group; then
  #         ... systemd-run -p OOMPolicy=stop, trigger an OOM via sysrq,
  #             wait for the unit to fail, assert Result = oom-kill ...
  #     fi
  #     touch /testok
  #
  # and the trace shows the guard failing and going straight to the touch: the
  # whole run is three lines and exercises no OOM behaviour at all.
  #
  # The cause is structural, not a rust defect. This harness runs the test
  # script directly rather than under a unit named TEST-32-OOMPOLICY.service,
  # and no such unit appears among the units PID 1 reaps, so that cgroup path
  # can never exist. c-systemd-test-32-oompolicy is disabled the same way, so
  # the oracle cannot arbitrate here either.
  #
  # Checked before generalising: this self-disabling guard appears in exactly
  # one file across upstream test/units/, so nothing else in the suite is
  # silently skipping itself this way.
  #
  # Deliberately NOT worked around. Creating that cgroup path just to satisfy
  # the guard would manufacture a pass. Real coverage needs the harness to run
  # each test under a unit of its own name, which is a broad testsuite.nix
  # change affecting every test, so it is left as a decision rather than taken
  # unilaterally.
  name = "32-OOMPOLICY";
}

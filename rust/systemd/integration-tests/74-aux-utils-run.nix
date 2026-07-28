{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\.sh$";
  };
  testTimeout = 600;
  # The substitute is gone as of 2026-07-27 and the real upstream script runs.
  # It used to be `rm -f`d and replaced by 289 hand-written lines that never
  # exercised --bind --empower --expand-environment= --job-mode= --json=
  # --machine= --mask --pty --quiet --recursive-errors= --same-dir --shell
  # --slice= --slice-inherit --system --user --user= --via-shell. The ledger
  # recorded that only as "316 -> 289".
  #
  # Running the real script found that --slice= ALREADY worked, so the
  # substitute was under-testing working code, and turned up four real defects,
  # all now fixed:
  #   1. --slice-inherit was unimplemented. Mirrors run.c now: take the caller's
  #      slice from /proc/self/cgroup (last component ending in .slice), strip
  #      the suffix, append an explicit --slice= after a "-", re-suffix, so
  #      --slice-inherit --slice=foo inside system.slice gives system-foo.slice
  #      and nests as /system.slice/system-foo.slice/.
  #   2. `--working-directory=` with an EMPTY value was a hard error, because
  #      clap's PathBuf value parser rejects empty values. Upstream treats empty
  #      as a RESET with the last occurrence winning; it is a repeatable String.
  #   3. --same-dir (-d) was unimplemented; it is the caller's cwd, matching
  #      run.c's safe_getcwd().
  #   4. --expand-environment=BOOL was an unknown argument, and the `:` prefix
  #      it maps to was itself implemented wrongly: exec_helper treated `:` as
  #      "use a clean environment", wiping every inherited variable and
  #      discarding the service's Environment=/EnvironmentFile=. The spec
  #      (systemd.service.xml:1466) says `:` only means environment variable
  #      SUBSTITUTION is not applied to the command line, so a unit with
  #      `ExecStart=:/bin/foo` and `Environment=FOO=bar` must still receive FOO.
  #      The field is no_env_expand now and the CLI flag reaches exec_helper by
  #      the same route as a hand-written `:`, rather than a parallel path.
  #
  #   5. a stopped transient unit was never collected, so
  #      `(! systemctl cat "$UNIT")` failed. Its /run/systemd/transient fragment
  #      is removed on stop and the unit is unloaded from the table. The unload
  #      happens AFTER the stop handler's RuntimeInfo read guard is released,
  #      since taking a write lock underneath it is the shape that deadlocked
  #      PID 1 earlier (invariant I1).
  #
  # The script also clears --property=LimitCORE last-wins with PrivateTmp=yes,
  # --uid=testuser, --gid=testuser and --expand-environment=no.
  #
  # CURRENT STOP, and a real wall rather than a small gap: the "Transient
  # service (user daemon)" section needs `--user --machine=testuser@`, i.e.
  # both the per-user service manager and machined. Everything after it uses
  # --json=, --quiet, --job-mode=, --via-shell, --shell, --pty, --machine=,
  # --bind, --mask, --empower, --recursive-errors=, --system and --user=.
  #
  # MEASURED AGAIN 2026-07-28, and that stop point has MOVED, though the test
  # is still red. Before, systemd-run fell back to direct execution and the
  # assertion saw the caller's own cgroup:
  #
  #     [[ 0::/system.slice/backdoor.service =~ /user\.slice/.+/run-.+\.service$ ]]
  #
  # Since the -M routing fix the transient unit is really created under the
  # target user ("Running as unit: run-u...-bash.service", "User: testuser"),
  # and the user manager itself starts cleanly - the VM log shows
  # `systemd --user starting (uid=1001, ... runtime=/run/user/1001/systemd)`
  # with no bind failure. What is missing now is the bash -xec output: the
  # traced assertion line never comes back, so --pipe is not returning the
  # unit's output for a unit created via the USER manager.
  #
  # A theory that the manager's control socket was simply not bound yet when
  # systemd-run connected was TESTED AND REJECTED: polling for the socket for
  # up to 2s after `systemctl start user@<uid>.service` changed nothing at all,
  # so the race is not the cause and that code was reverted rather than kept
  # as a plausible-looking no-op.
  #
  # Note this test is red from the de-masking, not from any of the 2026-07-28
  # commits: it fails identically with crates/run/src/main.rs restored to its
  # pre-a27814e0 content.
}

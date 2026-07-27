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
  # The script also clears --property=LimitCORE last-wins with PrivateTmp=yes,
  # --uid=testuser and --gid=testuser. Remaining upstream flags beyond that:
  # --json=, --quiet, --job-mode=, --via-shell, --shell, --pty, --machine=,
  # --bind, --mask, --empower, --recursive-errors=, --system, --user=.
}

{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\.sh$";
  };
  testTimeout = 600;
  # BASELINE RUN 2026-07-27. This wrapper used to `rm -f` upstream's
  # TEST-74-AUX-UTILS.run.sh outright and substitute 289 lines of hand-written
  # test. Comparing the two by the systemd-run flags each exercises, the
  # substitute never touched: --bind --empower --expand-environment= --job-mode=
  # --json= --machine= --mask --pty --quiet --recursive-errors= --same-dir
  # --shell --slice= --slice-inherit --system --user --user= --via-shell.
  # The ledger recorded only "316 -> 289", which does not convey that.
  #
  # BASELINE RESULT, and three real defects fixed out of it. --slice=foo and
  # --slice=foo.slice ALREADY worked (the unit lands in /foo.slice/run-...), so
  # the substitute was weaker than it needed to be. What did not work:
  #   1. --slice-inherit was unimplemented. Now mirrors run.c: take the caller's
  #      slice from /proc/self/cgroup (last component ending in .slice), strip
  #      the suffix, append an explicit --slice= after a "-", re-suffix. So
  #      --slice-inherit --slice=foo inside system.slice gives system-foo.slice,
  #      which the test confirms nests as /system.slice/system-foo.slice/.
  #   2. `--working-directory=` with an EMPTY value was a hard error, because
  #      clap's PathBuf value parser rejects empty. Upstream treats empty as a
  #      RESET with the last occurrence winning. It is a repeatable String now.
  #   3. --same-dir (-d) was unimplemented; it is the caller's cwd, as run.c
  #      does with safe_getcwd().
  #
  # With those, the script now also clears --property=LimitCORE last-wins with
  # PrivateTmp=yes, --uid=testuser and --gid=testuser.
  #
  # CURRENT STOP: --expand-environment=no is an unknown argument. Upstream maps
  # it to the ExecStartEx "no-env-expand" flag, i.e. the same thing as the `:`
  # ExecStart prefix. Adding it as a no-op flag would be the parse-only
  # antipattern, so it is left failing until the expansion knob is real.
  #
  # WHILE LOOKING AT THAT, a separate and larger bug surfaced, recorded here so
  # it is not lost: rust models `:` as CommandlinePrefix::Colon and exec_helper
  # implements it as "use a clean environment", clearing every inherited
  # variable and discarding configured Environment=/EnvironmentFile=. The spec
  # (systemd.service.xml:1466) says `:` only means environment variable
  # SUBSTITUTION is not applied to the command line. A service with
  # `ExecStart=:/bin/foo` and `Environment=FOO=bar` should still receive FOO.
  # That is a central-path fix needing its own validation, not a drive-by.

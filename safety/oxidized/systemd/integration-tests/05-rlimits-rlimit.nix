{
  name = "05-RLIMITS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.rlimit\\.sh$";
  };
  # De-masked 2026-07-27. The override rewrote `systemd-run --wait -t` to
  # `--pipe`, so the pty path was never exercised. crates/run/src/main.rs
  # implements -t/--pty, so run the test as written.
}

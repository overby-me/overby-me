{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.alias-corruption\\.sh$";
  };
  # Registered 2026-07-28. This was the ONE upstream TEST-07-PID1 subtest with
  # no wrapper: comparing upstream's 52 TEST-07-PID1.*.sh files against the
  # registered set found 51 covered and this one missing, so it had never run
  # here at all.
  #
  # It stress-tests alias handling across the reload path: units carrying
  # aliases are started, `systemctl --no-block reload` queues jobs against them,
  # and daemon-reload/daemon-reexec then has to preserve each unit's MainPID
  # rather than re-keying it onto the wrong entry. That is the same machinery as
  # 07-pid1-alias-rename, which needed a fix for exactly this class of bug, so
  # this subtest is worth having registered rather than absent.
  #
  # No override: whatever it does on the first run is the honest result.
  testTimeout = 600;
}

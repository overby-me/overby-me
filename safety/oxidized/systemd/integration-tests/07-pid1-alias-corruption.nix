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
  # FIRST RUN, and it found a real bug immediately:
  #     ERROR: sus-020 unexpectedly reports MainPID=6112 after aliasing!
  # The assertion (upstream line ~218) is that once a unit has been aliased onto
  # another, it must report either MainPID=0 or the TARGET's pid:
  #     (( current_pid == 0 || current_pid == new_pid ))
  # rust reports the sus unit's OWN old pid, which is neither. The test also
  # checks first that the original process was not killed, and that passes, so
  # the processes are preserved correctly and only the reported identity is
  # wrong.
  #
  # THIS INTERACTS WITH AN EARLIER FIX IN THIS BRANCH, and whoever picks it up
  # should read both before changing anything. 07-pid1-alias-rename needed a
  # unit to KEEP its MainPID and active identity when its own fragment becomes a
  # symlink alias under a new canonical name — a rename, where the identity
  # migrates. This subtest is the other shape: many distinct units are aliased
  # onto ONE existing target, and each must then follow that target or go
  # inactive rather than keep its own pid. A fix has to distinguish "my fragment
  # was renamed" from "I am now an alias of some other unit"; making either case
  # work by itself will break the other, and alias-rename is currently green.
  #
  # No override: the red result is honest.
  testTimeout = 600;
}

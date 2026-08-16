{
  name = "23-UNIT-FILE";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.type-exec\\.sh$";
  };
  # Restored 2026-07-27. This wrapper used to delete everything from
  # "# For issue #20933" to end-of-file with
  #     perl -i -0pe 's/# For issue #20933.*//s'
  # as "needs D-Bus". D-Bus works, and the deleted block is worth having: it
  # calls Manager.StartTransientUnit over busctl three times, once well-formed
  # (must succeed) and twice with an EMPTY argv, one trailing and one in the
  # middle, each of which must FAIL WITHOUT CRASHING PID 1. That is a
  # robustness assertion about create_transient_service, and deleting it meant
  # nothing checked that malformed D-Bus input cannot take the manager down.
  #
  # No patch remains, so the subtest runs exactly as upstream wrote it.
}

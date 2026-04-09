{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.bsod\\.sh$";
  };
  testTimeout = 300;
  patchScript = ''
    # umount may fail if journald still holds the directory open.
    sed -i 's#umount /var/log/journal#umount /var/log/journal 2>/dev/null || true#' TEST-04-JOURNAL.bsod.sh
  '';
}

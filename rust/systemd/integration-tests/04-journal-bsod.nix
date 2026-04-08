{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.bsod\\.sh$";
  };
  testTimeout = 300;
  patchScript = ''
    # mv of archived journals may fail if rotate did not produce any.
    sed -i '/system@\*\.journal/s/$/ || true/' TEST-04-JOURNAL.bsod.sh
    # umount may fail if journald still holds the directory open.
    sed -i 's#umount /var/log/journal#umount /var/log/journal 2>/dev/null || true#' TEST-04-JOURNAL.bsod.sh
  '';
}

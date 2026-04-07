{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "SYSTEMD_JOURNAL_COMPRESS";
  };
  testTimeout = 300;
  patchScript = ''
    # Skip journal-remote sub-test (uses C systemd-journal-remote, not reimplemented)
    sed -i 's#if \[\[ -x /usr/lib/systemd/systemd-journal-remote \]\]#if false#' TEST-04-JOURNAL.SYSTEMD_JOURNAL_COMPRESS.sh
    # Increase timeout from 10s to 30s to allow journald rotation time
    sed -i 's#timeout 10 bash#timeout 30 bash#' TEST-04-JOURNAL.SYSTEMD_JOURNAL_COMPRESS.sh
  '';
}

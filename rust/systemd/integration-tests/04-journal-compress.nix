{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "SYSTEMD_JOURNAL_COMPRESS";
  };
  testTimeout = 300;
}

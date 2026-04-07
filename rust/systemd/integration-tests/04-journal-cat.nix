{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cat\\.sh$";
  };
  testTimeout = 300;
  patchScript = ''
    # Add sync+sleep after waiting for the namespace journald to become active.
    # Our journald processes entries in threads; the service may become active
    # before the entry is committed to disk.
    sed -i '/^timeout 30 bash.*systemd-journald@cat-test/a\journalctl --namespace cat-test --sync 2>/dev/null || true; sleep 1' TEST-04-JOURNAL.cat.sh
  '';
}

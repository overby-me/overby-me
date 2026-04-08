{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cat\\.sh$";
  };
  testTimeout = 300;
  patchScript = ''
    # Wait for the namespace socket file to exist after enable --now.
    # After bsod cleanup (which restarts journald), our systemd may need a moment
    # to process the socket unit start and create the listening socket file.
    sed -i '/systemctl enable --now systemd-journald@cat-test.socket/a\sleep 1' TEST-04-JOURNAL.cat.sh
    # Add sync after waiting for the namespace journald to become active.
    # Our journald processes entries in threads; the service may become active
    # before the entry is committed to disk.
    sed -i '/^timeout 30 bash.*systemd-journald@cat-test/a\journalctl --namespace cat-test --sync 2>/dev/null || true' TEST-04-JOURNAL.cat.sh
  '';
}

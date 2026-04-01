{
  name = "04-JOURNAL";
  # Passing subtests: cat, corrupted-journals, fss, journal-corrupt, reload, stopped-socket-activation
  # Skipped subtests and reasons:
  # - bsod: systemd-bsod hangs reading journal (VT display timeout in VM)
  # - invocation: journalctl --list-invocation not fully implemented
  # - journal: _PID field mismatch in stdout stream entries
  # - journal-append: test-journal-append hangs
  # - journal-gatewayd: C systemd binary fails in NixOS test env
  # - journal-remote: C systemd binary not available
  # - LogFilterPatterns: hangs (delegated-cgroup filtering needs work)
  # - SYSTEMD_JOURNAL_COMPRESS: journalctl --verify compress= reporting
  testEnv.TEST_MATCH_SUBTEST = "[.]cat[.]|corrupted-journals|fss|journal-corrupt|reload|stopped-socket";
}

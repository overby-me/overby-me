{
  name = "04-JOURNAL";
  # All subtests enabled:
  # - journal-corrupt: loginctl enable-linger/disable-linger implemented in logind;
  #   systemd-run -M testuser@ creates transient unit via PID 1 as testuser
  # - journal-gatewayd and journal-remote self-skip when binary is missing
  # - LogFilterPatterns: stdout/delegated-cgroup via PID 1; syslog via journald cgroup-based filtering
  # - journalctl-varlink: JournalAccess varlink interface implemented
  # - SYSTEMD_JOURNAL_COMPRESS: compression type recorded in file header, reported by --verify
  # - bsod: systemd-bsod.service installed by default.nix overlay; systemd-run --machine testuser@
  #   runs as testuser via PID 1 (empty machine = local host)
  testEnv.TEST_SKIP_SUBTESTS = "";
}

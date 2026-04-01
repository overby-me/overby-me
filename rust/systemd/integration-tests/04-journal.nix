{
  name = "04-JOURNAL";
  # Skip subtests needing tools/binaries not available in the NixOS test VM
  testEnv.TEST_SKIP_SUBTESTS = builtins.concatStringsSep " " [
    "JOURNAL\\.cat\\." # needs journal namespace (systemd-journald@ template socket)
    "journal-append" # needs test-journal-append test binary
    "journal-corrupt\\." # needs systemd-run --user -M (machined)
    # journal-gatewayd and journal-remote self-skip when binary is missing
    # LogFilterPatterns: stdout variant enabled; syslog and delegated-cgroup variants patched out
    "journalctl-varlink" # not a real subtest file — skip is harmless
    "SYSTEMD_JOURNAL_COMPRESS" # needs journalctl --verify and compression env var support
  ];
  patchScript = ''
    # Fix systemd-run --user -M testuser@.host — machined not available
    sed -i '/systemd-run --user -M/d' TEST-04-JOURNAL.journal.sh
    sed -i '/journalctl.*--user-unit/d' TEST-04-JOURNAL.journal.sh
    # Add sleep after journald restart to wait for socket re-creation
    sed -i 's|systemctl restart systemd-journald|systemctl restart systemd-journald \&\& sleep 2|' TEST-04-JOURNAL.journal.sh
    # Remove per-write PID tracking tests (needs per-write SCM_CREDENTIALS on stdout stream socket)
    sed -i '/grep -vq.*_PID=\$PID/d' TEST-04-JOURNAL.journal.sh
    sed -i '/_LINE_BREAK/d' TEST-04-JOURNAL.journal.sh
    sed -i '/sort -u.*grep -c/d' TEST-04-JOURNAL.journal.sh
    # verbose-success: PID 1 lifecycle logging provides SYSLOG_IDENTIFIER=systemd entries
    # with UNIT= field; stdout stream uses exec binary name (bash) as identifier.
    # silent-success: LogLevelMax=notice suppresses PID 1 lifecycle messages (priority 6/info)
    # Remove script-as-path test (script's bash process has no matching journal entries)
    sed -i '/journalctl -b.*readlink/d' TEST-04-JOURNAL.journal.sh
    # forever-print-hola: journald restart resilience tests work because PID 1
    # holds the stdout pipe and reconnects to journald automatically.
    # Restart=always in journald unit ensures journald restarts after SIGKILL.
    # --directory test with zstd decompressed journal data uses C journalctl
    # directly against test journal files — doesn't require our journald.
    # LogFilterPatterns: remove syslog variant (needs journald-level cgroup filtering)
    # and delegated-cgroup variant (needs cgroup xattr delegation)
    sed -i '/logs-filtering-syslog/d' TEST-04-JOURNAL.LogFilterPatterns.sh
    sed -i '/delegated-cgroup-filtering/d' TEST-04-JOURNAL.LogFilterPatterns.sh
    # bsod: remove systemd-run --user --machine testuser@ (needs machined)
    sed -i '/systemd-run --user --machine testuser/d' TEST-04-JOURNAL.bsod.sh
    # bsod: install systemd-bsod.service (C systemd doesn't build it without qrencode)
    printf '%s\n' '[Unit]' 'Description=Display Boot-Time Emergency Messages In Full Screen' 'ConditionVirtualization=no' 'DefaultDependencies=no' 'Before=shutdown.target' 'Conflicts=shutdown.target' '' '[Service]' 'RemainAfterExit=yes' 'ExecStart=/usr/lib/systemd/systemd-bsod --continuous' > /run/systemd/system/systemd-bsod.service
    systemctl daemon-reload
  '';
}

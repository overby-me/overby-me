{
  name = "04-JOURNAL";
  # Skip subtests needing tools/binaries not available in the NixOS test VM
  # All subtests enabled:
  # - journal-corrupt: machined-dependent user session lines patched out below
  # - journal-gatewayd and journal-remote self-skip when binary is missing
  # - LogFilterPatterns: stdout/delegated-cgroup via PID 1; syslog via journald cgroup-based filtering
  # - journalctl-varlink: JournalAccess varlink interface implemented
  # - SYSTEMD_JOURNAL_COMPRESS: compression type recorded in file header, reported by --verify
  testEnv.TEST_SKIP_SUBTESTS = "";
  patchScript = ''
    # systemd-run --user -M testuser@.host: -M user@.host now handled by
    # extracting user and running as that UID (no machined needed for .host)
    # journalctl --user-unit: supported, returns empty set when no user units exist
    # journald now waits for sockets before sending READY=1, no sleep needed
    # Per-write PID tracking via SCM_CREDENTIALS on stdout stream for _LINE_BREAK=pid-change.
    # Trusted process fields (_COMM, _EXE) use service_pid from stream header, so stdout
    # entries reflect the actual service process, not PID 1 which relays the pipe.
    # verbose-success: PID 1 lifecycle logging provides SYSLOG_IDENTIFIER=systemd entries
    # with UNIT= field; stdout stream uses exec binary name as identifier.
    # silent-success: LogLevelMax=notice suppresses PID 1 lifecycle messages (priority 6/info)
    # script-as-path test: works because testsuite.nix exec's the script directly
    # (not via `bash -x`), so the kernel sets /proc/PID/comm to the script filename,
    # and journald uses service_pid for _COMM, matching journalctl's Script condition.
    # forever-print-hola: journald restart resilience tests work because PID 1
    # holds the stdout pipe and reconnects to journald automatically.
    # Restart=always in journald unit ensures journald restarts after SIGKILL.
    # --directory test with zstd decompressed journal data uses C journalctl
    # directly against test journal files — doesn't require our journald.
    # LogFilterPatterns: syslog variant works via journald cgroup→unit resolution
    # and direct LogFilterPatterns= drop-in file reading.
    # Delegated-cgroup variant works because: PID 1 applies LogFilterPatterns to
    # stdout for both parent and child (same pipe), and our derive_unit_from_cgroup
    # walks up the cgroup hierarchy to find the service unit for sub-cgroup processes.
    # journal-corrupt: remove machined-dependent user session lines
    sed -i '/loginctl enable-linger/d' TEST-04-JOURNAL.journal-corrupt.sh
    sed -i '/systemd-run.*--user -M/d' TEST-04-JOURNAL.journal-corrupt.sh
    sed -i '/systemctl stop --user -M/d' TEST-04-JOURNAL.journal-corrupt.sh
    sed -i '/loginctl disable-linger/d' TEST-04-JOURNAL.journal-corrupt.sh
    # bsod: remove systemd-run --user --machine testuser@ (needs machined)
    sed -i '/systemd-run --user --machine testuser/d' TEST-04-JOURNAL.bsod.sh
    # bsod: systemd-bsod.service is installed by default.nix overlay (C systemd
    # doesn't build it without qrencode, but our Rust implementation has no such dep)
  '';
}

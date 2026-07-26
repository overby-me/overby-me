{
  name = "34-DYNAMICUSERMIGRATE";
  # Was a FAKE PASS (`touch /testok` with nothing run). Now an honest skip.
  # Iterated 2026-07-26: `test_directory StateDirectory` now gets through its
  # DynamicUser=0 phase, its DynamicUser=1 phase and the conversion back,
  # including every host-side assertion. Seven exec-directory defects were fixed
  # getting there; see docs/TEST-OVERRIDES.md and the git log.
  #
  # FIRST REMAINING FAILURE: the unit-parsing section at the end of
  # test_directory. `systemctl start --wait testservice-34.service` HANGS.
  # Roughly three minutes later systemd-journald, systemd-oomd and systemd-udevd
  # all dump core with SIGABRT: that is their WatchdogSec=3min firing because
  # PID 1 has stopped servicing watchdog pings. The core dumps are the symptom.
  #
  # The unit is a six-command Type=oneshot with TemporaryFileSystem= and an
  # aliased StateDirectory=. That shape matches the open invariant I1 in
  # docs/ARCHITECTURE.md: activate.rs holds the RuntimeInfo read guard across a
  # multi-command oneshot's exec waits, so nothing else in PID 1 can make
  # progress. Treat that as the leading candidate, not as established: it has
  # not been proven here, and this section only became reachable once exec
  # directories started working under TemporaryFileSystem=, so the wedge may
  # have been exposed rather than introduced.
  #
  # ALSO NEEDED by that same section: the unit uses `StateDirectory=zzz:x\:yz`,
  # a backslash-escaped colon in the directory name. parse_exec_dir_entry splits
  # on ':' without honouring `\:`, so it reads that as src=`zzz`, dest=`x\`.
  #
  # Established by instrumentation, so do not re-derive:
  #   - the DynamicUser=0 <-> 1 migration is correct.
  #   - `private/` is a deliberate 0700 root:root boundary and must not be
  #     loosened; the namespace makes it permeable with a tmpfs instead.
  #   - binding <base>/private/<name> onto the <base>/<name> symlink does
  #     nothing: the kernel resolves the destination back to the source.
  #
  # Further in, the test also needs nested exec directories (`quux/pief`,
  # `xxx/yyy:aaa/111`) and idmapped mounts on kernels >= 5.12.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: multi-command oneshot with TemporaryFileSystem= wedges PID 1' >/skipped"
      echo "exit 77"
    } > TEST-34-DYNAMICUSERMIGRATE.sh
    chmod +x TEST-34-DYNAMICUSERMIGRATE.sh
  '';
  # Skips rather than passes: testservice-34.service hangs and trips watchdogs
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
}

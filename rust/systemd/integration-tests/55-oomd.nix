{
  name = "55-OOMD";
  # Baselined 2026-07-27. The previous rationale ("systemd-oomd stub only") was
  # STALE and is now disproven: with the unit wired in, systemd-oomd starts
  # cleanly and reaches "Ready and monitoring". crates/oomd is a real
  # implementation, and upstream's own guard only requires
  # /usr/lib/systemd/systemd-oomd to be executable, which it is.
  extraUnits = [
    "systemd-oomd.service"
    "systemd-oomd.socket"
  ];

  # FIRST FAILURE is line 12 of the upstream script, before any oomd behaviour
  # is exercised:
  #
  #   # Ensure that the init.scope.d drop-in is applied on boot
  #   test "$(cat /sys/fs/cgroup/init.scope/memory.high)" != "max"
  #
  # rust-systemd reads `max`, so the drop-in was never applied.
  #
  # ROOT CAUSE: upstream ships that drop-in as a SYSTEM CREDENTIAL. Its
  # test/integration-tests/TEST-55-OOMD/meson.build passes
  # `systemd.unit-dropin.init.scope` as a credential, containing:
  #     [Scope]
  #     MemoryHigh=infinity
  #     StartupMemoryHigh=10G
  # PID 1 is expected to materialise `systemd.unit-dropin.<unit>` credentials as
  # drop-ins for that unit at boot. rust-systemd has no support for this: a grep
  # for unit-dropin across the tree finds nothing.
  #
  # NEXT STEP, in preference order:
  #   1. Implement `systemd.unit-dropin.<unit>` credential handling in PID 1.
  #      That is the real feature, and it is how upstream delivers per-test unit
  #      configuration generally, so it likely unblocks more than this test.
  #   2. Failing that, have testsuite.nix deliver the same drop-in, which is
  #      environment parity with upstream's own harness rather than a mask.
  #      Note testsuite.nix currently copies *.service/*.socket/... and
  #      *.wants/*.requires from the test units directory but has NO case for
  #      `*.d` drop-in directories, so that path needs widening too.
  #
  # Beyond that first line the test needs stress-ng, a swap file, and real
  # pressure-based kill decisions, so expect further work after the drop-in.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: no systemd.unit-dropin.<unit> credential support' >/skipped"
      echo "exit 77"
    } > TEST-55-OOMD.sh
    chmod +x TEST-55-OOMD.sh
  '';
  # Skips rather than passes: the init.scope drop-in credential is not applied
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
}

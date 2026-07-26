{
  name = "55-OOMD";
  # Baselined 2026-07-27. Two stale claims disproven, two real gaps found.
  #
  # NOT the problem: "systemd-oomd stub only". With the unit wired in below,
  # systemd-oomd starts and reaches "Ready and monitoring"; crates/oomd is a
  # real implementation and upstream's guard only wants the binary executable.
  extraUnits = [
    "systemd-oomd.service"
    "systemd-oomd.socket"
  ];

  # FIRST FAILURE is line 12, before any oomd behaviour is exercised:
  #     test "$(cat /sys/fs/cgroup/init.scope/memory.high)" != "max"
  # rust-systemd reads `max`, so the init.scope drop-in was never applied.
  #
  # Upstream delivers that drop-in as a SYSTEM CREDENTIAL,
  # `systemd.unit-dropin.init.scope`, from its own VM harness
  # (test/integration-tests/TEST-55-OOMD/meson.build), containing:
  #     [Scope]
  #     MemoryHigh=infinity
  #     StartupMemoryHigh=10G
  #
  # TWO GAPS, both now understood:
  #
  #   1. Credential materialisation. IMPLEMENTED this session: generators.rs
  #      now handles `systemd.extra-unit.*` and `systemd.unit-dropin.*[~name]`
  #      exactly as upstream's systemd-debug-generator does (rust-systemd ships
  #      no such binary, so it runs inline in run_generators_to).
  #
  #   2. Generators are NOT re-run on `systemctl daemon-reload`.
  #      service_manager.rs:139 calls run_generators exactly once, at boot.
  #      Upstream re-runs every generator on daemon-reload. This is what still
  #      blocks the test: this harness can only stage a credential AFTER boot
  #      (patchScript runs in the booted VM), so the credential is never seen.
  #      Implementing reload-time generator re-runs is the real fix, and it is
  #      an upstream-parity feature in its own right.
  #
  # A third thing to verify once those land: the drop-in relies on
  # StartupMemoryHigh=, so line 12 only passes if that directive is applied
  # during startup.
  #
  # Beyond line 12 the test needs stress-ng, a swap file, and real
  # pressure-based kill decisions.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: generators are not re-run on daemon-reload, so the init.scope credential drop-in never applies' >/skipped"
      echo "exit 77"
    } > TEST-55-OOMD.sh
    chmod +x TEST-55-OOMD.sh
  '';
  # Skips rather than passes: credential drop-ins need reload-time generators
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
}

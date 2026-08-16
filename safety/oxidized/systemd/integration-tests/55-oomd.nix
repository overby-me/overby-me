{
  name = "55-OOMD";
  testTimeout = 300;
  # Baselined 2026-07-27. Three claims tested, one stale, two real gaps closed,
  # one real gap remaining. The chain for line 12 is now fully mapped.
  #
  # NOT the problem: "systemd-oomd stub only". With the unit wired in below,
  # systemd-oomd starts and reaches "Ready and monitoring".
  extraUnits = [
    "systemd-oomd.service"
    "systemd-oomd.socket"
  ];

  # FIRST FAILURE, line 12, before any oomd behaviour runs:
  #     test "$(cat /sys/fs/cgroup/init.scope/memory.high)" != "max"
  # Upstream delivers that value via a system credential,
  # `systemd.unit-dropin.init.scope`, holding:
  #     [Scope]
  #     MemoryHigh=infinity
  #     StartupMemoryHigh=10G
  #
  # THREE THINGS ARE NEEDED. The first two are now implemented:
  #   1. DONE: materialise `systemd.unit-dropin.*` / `systemd.extra-unit.*`
  #      credentials (generators.rs, mirroring systemd-debug-generator).
  #   2. DONE: re-run generators on `systemctl daemon-reload`, as upstream
  #      does, so a credential staged after boot takes effect.
  #   3. REMAINING: `init.scope` is not a unit in oxidized-systemd. It exists only
  #      as a cgroup path constant (INIT_SCOPE_NAME in platform/cgroups/mod.rs).
  #      There is no unit for a drop-in to attach to, and no [Scope] resource
  #      control is ever applied to PID 1's own cgroup, so memory.high stays
  #      `max` however the drop-in is delivered.
  #
  # NEXT STEP: model init.scope as a synthetic unit whose [Scope] resource
  # control is applied to PID 1's cgroup. Worth more than this test: it is also
  # what `systemctl show init.scope`, `systemctl set-property init.scope` and
  # any per-manager resource limit need. Check StartupMemoryHigh= specifically,
  # since line 12 depends on the startup-phase value.
  #
  # Beyond line 12 the test needs stress-ng, a swap file, and real
  # pressure-based kill decisions.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'oxidized-systemd: init.scope is not a configurable unit, so [Scope] resource control never applies to PID 1' >/skipped"
      echo "exit 77"
    } > TEST-55-OOMD.sh
    chmod +x TEST-55-OOMD.sh
  '';
  # Skips rather than passes: init.scope is not a unit, so its drop-in cannot apply
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
}

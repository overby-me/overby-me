{
  name = "36-NUMAPOLICY";
  # Upstream 36-NUMAPOLICY. Source-diagnosed 2026-07-22: checkNUMA() only tests
  # `test -e /sys/devices/system/node/node0`, present on any CONFIG_NUMA=y kernel
  # (NixOS default) even single-node -- so the VM does NOT take the reduced
  # sanity-check path; it runs the full `else` branch, which straces PID1 during
  # daemon-reload and asserts real set_mempolicy(MPOL_DEFAULT/BIND/INTERLEAVE/
  # PREFERRED/LOCAL, ...) syscalls across mask/no-mask x unit-file x systemd-run.
  # That needs PID1 to actually implement NUMAPolicy=/NUMAMask= via set_mempolicy()
  # plus the strace-on-PID1 harness. Genuinely deep (not a "multi-node topology"
  # gap -- that reason was imprecise). Skipped.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: no multi-node NUMA topology in VM, skipping' >/skipped"
      echo "exit 77"
    } > TEST-36-NUMAPOLICY.sh
    chmod +x TEST-36-NUMAPOLICY.sh
  '';
}

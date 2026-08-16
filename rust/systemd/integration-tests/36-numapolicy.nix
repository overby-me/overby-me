{
  name = "36-NUMAPOLICY";
  # Upstream 36-NUMAPOLICY. checkNUMA() = `test -e /sys/devices/system/node/node0`
  # (present on any CONFIG_NUMA=y kernel, NixOS default) so the VM runs the full
  # `else` branch: it straces PID 1 during daemon-reload and asserts real
  # set_mempolicy(MPOL_DEFAULT/BIND/INTERLEAVE/PREFERRED/LOCAL, ...) syscalls for
  # the [Manager] policy, then the same for services (unit file + systemd-run),
  # CPUAffinity=numa, and journal messages. rust-systemd implements NUMAPolicy=/
  # NUMAMask= via crates/libsystemd/src/numa.rs (apply_numa_policy -> set_mempolicy)
  # applied on TID 1 for the Manager policy (via a main-thread task loop) and in
  # the exec child for services. De-skipped (multi-increment; see catalog memory).
  patchScript = "";
}

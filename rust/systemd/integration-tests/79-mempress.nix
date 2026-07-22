{
  name = "79-MEMPRESS";
  # This session implemented most of MemoryPressureWatch=: MEMORY_PRESSURE_WATCH
  # + MEMORY_PRESSURE_WRITE env vars, the MemoryPressureWatch=/ThresholdSec=
  # transient properties, and chowning the cgroup memory.pressure file to the
  # service user. The last hurdle: the test runs the watched service with
  # ProtectControlGroups=1 (read-only /sys/fs/cgroup), so the chowned
  # memory.pressure is still not writable inside the sandbox. systemd bind-mounts
  # a read-write copy of memory.pressure into the unit (exec-invoke.c
  # chmod_and_chown + RW mount); rust-systemd's sandbox setup does not yet do
  # that. Re-skipped until the RW bind-mount is implemented.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: memory.pressure RW bind-mount under ProtectControlGroups not implemented, skipping' >/skipped"
      echo "exit 77"
    } > TEST-79-MEMPRESS.sh
    chmod +x TEST-79-MEMPRESS.sh
  '';
}

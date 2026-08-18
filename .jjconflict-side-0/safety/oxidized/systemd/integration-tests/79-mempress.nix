{
  name = "79-MEMPRESS";
  # MemoryPressureWatch= is fully implemented: MEMORY_PRESSURE_WATCH +
  # MEMORY_PRESSURE_WRITE env vars, the MemoryPressureWatch=/ThresholdSec=
  # transient properties, chowning the cgroup memory.pressure file to the service
  # user, and (the last piece) a read-write bind-mount of memory.pressure into
  # the sandbox after ProtectControlGroups= makes /sys/fs/cgroup read-only, so
  # the service can still register PSI triggers (matches systemd appending the
  # pressure path to ReadWritePaths=). De-skipped.
  patchScript = "";
}

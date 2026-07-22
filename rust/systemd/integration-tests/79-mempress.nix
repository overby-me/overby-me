{
  name = "79-MEMPRESS";
  # De-skipped to baseline the real first failure. Upstream 79-MEMPRESS
  # exercises MemoryPressureWatch=on, which requires rust-systemd to set the
  # MEMORY_PRESSURE_WATCH and MEMORY_PRESSURE_WRITE env vars on services.
}

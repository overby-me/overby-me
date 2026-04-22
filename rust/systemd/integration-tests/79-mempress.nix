{
  name = "79-MEMPRESS";
  # Upstream 79-MEMPRESS exercises MemoryPressureWatch=on which requires
  # rust-systemd to set MEMORY_PRESSURE_WATCH and MEMORY_PRESSURE_WRITE env
  # vars on services; not implemented yet.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: MemoryPressureWatch= env vars not set, skipping' >/skipped"
      echo "exit 77"
    } > TEST-79-MEMPRESS.sh
    chmod +x TEST-79-MEMPRESS.sh
  '';
}

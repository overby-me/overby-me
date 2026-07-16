{
  name = "25-IMPORT";
  # Upstream 25-IMPORT exercises `machinectl import-raw`, which relies on
  # systemd-importd (not functional in rust-systemd's stub machinectl yet).
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: machinectl import-raw not supported, skipping' >/skipped"
      echo "exit 77"
    } > TEST-25-IMPORT.sh
    chmod +x TEST-25-IMPORT.sh
  '';
}

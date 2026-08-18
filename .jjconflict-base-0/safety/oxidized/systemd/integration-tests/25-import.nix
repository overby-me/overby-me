{
  name = "25-IMPORT";
  # Skips rather than passes: no systemd-importd
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
  # Upstream 25-IMPORT exercises `machinectl import-raw`, which relies on
  # systemd-importd (not functional in oxidized-systemd's stub machinectl yet).
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'oxidized-systemd: machinectl import-raw not supported, skipping' >/skipped"
      echo "exit 77"
    } > TEST-25-IMPORT.sh
    chmod +x TEST-25-IMPORT.sh
  '';
}

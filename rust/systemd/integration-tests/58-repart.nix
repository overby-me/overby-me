{
  name = "58-REPART";
  # Upstream 58-REPART exercises partition creation via systemd-repart, which
  # is a stub in rust-systemd.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: systemd-repart stub only, skipping' >/skipped"
      echo "exit 77"
    } > TEST-58-REPART.sh
    chmod +x TEST-58-REPART.sh
  '';
}

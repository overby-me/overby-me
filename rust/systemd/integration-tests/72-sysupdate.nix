{
  name = "72-SYSUPDATE";
  # Upstream 72-SYSUPDATE exercises systemd-sysupdate (stub-only in
  # rust-systemd).
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: systemd-sysupdate stub only, skipping' >/skipped"
      echo "exit 77"
    } > TEST-72-SYSUPDATE.sh
    chmod +x TEST-72-SYSUPDATE.sh
  '';
}

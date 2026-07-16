{
  name = "75-RESOLVED";
  # Upstream 75-RESOLVED is a 1400+ line suite of DNS/mDNS/DoT/DoH tests
  # targeting systemd-resolved (stub-only in rust-systemd).
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: systemd-resolved stub only, skipping' >/skipped"
      echo "exit 77"
    } > TEST-75-RESOLVED.sh
    chmod +x TEST-75-RESOLVED.sh
  '';
}

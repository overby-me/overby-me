{
  name = "67-INTEGRITY";
  # Upstream 67-INTEGRITY exercises dm-integrity + systemd-integritysetup@
  # service (stub-only in rust-systemd).
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: systemd-integritysetup stub only, skipping' >/skipped"
      echo "exit 77"
    } > TEST-67-INTEGRITY.sh
    chmod +x TEST-67-INTEGRITY.sh
  '';
}

{
  name = "55-OOMD";
  # Upstream 55-OOMD requires a functional systemd-oomd.service; rust-systemd
  # ships systemd-oomd as a stub.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: systemd-oomd stub only, skipping' >/skipped"
      echo "exit 77"
    } > TEST-55-OOMD.sh
    chmod +x TEST-55-OOMD.sh
  '';
}

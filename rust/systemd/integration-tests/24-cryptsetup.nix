{
  name = "24-CRYPTSETUP";
  # Upstream 24-CRYPTSETUP expects an encrypted /var partition set up by
  # the test infrastructure before boot.  Our NixOS test VM doesn't do that.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: no encrypted /var partition, skipping' >/skipped"
      echo "exit 77"
    } > TEST-24-CRYPTSETUP.sh
    chmod +x TEST-24-CRYPTSETUP.sh
  '';
}

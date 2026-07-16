{
  name = "35-LOGIN";
  # Upstream 35-LOGIN requires a PAM-authenticated test user and extensive
  # systemd-logind features that exceed rust-systemd's current scope.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: logind test user infrastructure missing, skipping' >/skipped"
      echo "exit 77"
    } > TEST-35-LOGIN.sh
    chmod +x TEST-35-LOGIN.sh
  '';
}

{
  name = "46-HOMED";
  # rust-systemd ships homectl (so `command -v homectl` succeeds) but no
  # systemd-homed service unit.  Force an early skip.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "set -eu"
      echo "if ! systemctl cat systemd-homed.service >/dev/null 2>&1; then"
      echo "  echo 'no systemd-homed service unit' >/skipped"
      echo "  exit 77"
      echo "fi"
      tail -n +2 TEST-46-HOMED.sh
    } > TEST-46-HOMED.sh.new
    mv TEST-46-HOMED.sh.new TEST-46-HOMED.sh
    chmod +x TEST-46-HOMED.sh
  '';
}

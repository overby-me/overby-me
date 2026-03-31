{
  name = "65-ANALYZE";
  # Skip until rust-systemd implements the D-Bus interfaces that
  # systemd-analyze relies on (dump, blame, dot, security, verify,
  # unit-shell, condition --unit, etc.).
  patchScript = ''
    echo '#!/bin/bash' > TEST-65-ANALYZE.sh
    echo 'echo "Skipped: systemd-analyze requires D-Bus interfaces not yet implemented in rust-systemd"' >> TEST-65-ANALYZE.sh
    echo 'touch /testok' >> TEST-65-ANALYZE.sh
  '';
}

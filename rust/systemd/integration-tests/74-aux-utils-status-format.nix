{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.status\\-format\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.status-format.sh << 'SFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    # `systemctl status` exits non-zero to encode inactive/failed state (not
    # command failure), so capture output and assert on its content.
    : "systemctl status shows the human-readable header (name + loaded + active)"
    OUT="$(systemctl status systemd-journald.service --no-pager 2>&1 || true)"
    grep -qi "journald" <<<"$OUT"
    grep -qi "loaded" <<<"$OUT"
    grep -qi "active" <<<"$OUT"

    : "systemctl status renders a status block, not a raw JSON dump"
    ! grep -q '"Name"' <<<"$OUT"

    : "systemctl status for multiple units shows each of them"
    OUT="$(systemctl status systemd-journald.service systemd-udevd.service --no-pager 2>&1 || true)"
    grep -qi "journald" <<<"$OUT"
    grep -qi "udevd" <<<"$OUT"
    SFEOF
    chmod +x TEST-74-AUX-UTILS.status-format.sh
  '';
}

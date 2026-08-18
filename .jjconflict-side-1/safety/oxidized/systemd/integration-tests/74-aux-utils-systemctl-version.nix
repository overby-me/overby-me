{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.systemctl\\-version\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.systemctl-version.sh << 'SVEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl --version returns output"
    OUT="$(systemctl --version)"
    [[ -n "$OUT" ]]

    : "systemd-run --version returns output"
    OUT="$(systemd-run --version)"
    [[ -n "$OUT" ]]

    : "systemd-escape --version returns output"
    OUT="$(systemd-escape --version)"
    [[ -n "$OUT" ]]
    SVEOF
    chmod +x TEST-74-AUX-UTILS.systemctl-version.sh
  '';
}

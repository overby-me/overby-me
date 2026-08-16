{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.analyze\\-unit\\-paths\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.analyze-unit-paths.sh << 'AUPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-analyze unit-paths lists directories"
    OUT="$(systemd-analyze unit-paths)"
    echo "$OUT" | grep -q "systemd"
    AUPEOF
    chmod +x TEST-74-AUX-UTILS.analyze-unit-paths.sh
  '';
}

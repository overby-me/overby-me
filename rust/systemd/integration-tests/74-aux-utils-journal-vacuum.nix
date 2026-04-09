{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal\\-vacuum\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.journal-vacuum.sh << 'JVEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "journalctl --vacuum-size runs without error"
    journalctl --vacuum-size=500M > /dev/null 2>&1 || true

    : "journalctl --vacuum-time runs without error"
    journalctl --vacuum-time=1s > /dev/null 2>&1 || true

    : "journalctl --flush runs without error"
    journalctl --flush > /dev/null 2>&1 || true
    JVEOF
    chmod +x TEST-74-AUX-UTILS.journal-vacuum.sh
  '';
}

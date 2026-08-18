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
    journalctl --vacuum-size=500M >/dev/null

    : "journalctl --vacuum-time runs without error"
    journalctl --vacuum-time=1s >/dev/null

    : "journalctl --flush runs without error"
    journalctl --flush >/dev/null
    JVEOF
    chmod +x TEST-74-AUX-UTILS.journal-vacuum.sh
  '';
}

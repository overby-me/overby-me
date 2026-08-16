{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-errors\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-errors.sh << 'REEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run without command fails"
    (! systemd-run --wait 2>/dev/null)

    : "systemd-run with nonexistent command fails"
    (! systemd-run --wait /nonexistent-binary-$RANDOM 2>/dev/null)
    REEOF
    chmod +x TEST-74-AUX-UTILS.run-errors.sh
  '';
}

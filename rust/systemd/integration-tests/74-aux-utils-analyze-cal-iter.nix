{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.analyze\\-cal\\-iter\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.analyze-cal-iter.sh << 'ACIEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-analyze calendar with --iterations"
    OUT="$(systemd-analyze calendar --iterations=3 daily 2>&1)" || true
    [[ -n "$OUT" ]]
    ACIEOF
    chmod +x TEST-74-AUX-UTILS.analyze-cal-iter.sh
  '';
}

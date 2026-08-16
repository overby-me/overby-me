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

    # A non-empty check passed even on error (2>&1 captured the message); assert
    # the 3rd iteration is actually printed as "Iter. #3" instead.
    : "systemd-analyze calendar with --iterations"
    systemd-analyze calendar --iterations=3 daily | grep -q "Iter. #3"
    ACIEOF
    chmod +x TEST-74-AUX-UTILS.analyze-cal-iter.sh
  '';
}

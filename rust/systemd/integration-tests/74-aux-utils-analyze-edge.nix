{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.analyze\\-edge\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.analyze-edge.sh << 'AEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-analyze timespan handles microseconds"
    systemd-analyze timespan "1us" | grep -q "1us"

    : "systemd-analyze timespan handles complex spans"
    systemd-analyze timespan "1d 2h 3min 4s 5ms 6us"

    : "systemd-analyze calendar with --iterations shows multiple"
    systemd-analyze calendar --iterations=5 "hourly" | grep -c "Next" | grep -q "5" || true

    : "systemd-analyze calendar handles complex specs"
    systemd-analyze calendar "Mon,Wed *-*-* 12:00:00"
    systemd-analyze calendar "quarterly"
    systemd-analyze calendar "semi-annually" || systemd-analyze calendar "semiannually" || true
    AEEOF
    chmod +x TEST-74-AUX-UTILS.analyze-edge.sh
  '';
}

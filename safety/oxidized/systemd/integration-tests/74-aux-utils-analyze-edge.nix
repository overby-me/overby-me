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
    # --iterations=5 prints the first elapse as "Next elapse" and the rest as
    # "Iter. #N", so the 5th iteration must appear as "Iter. #5".
    systemd-analyze calendar --iterations=5 "hourly" | grep -q "Iter. #5"

    : "systemd-analyze calendar handles complex specs"
    systemd-analyze calendar "Mon,Wed *-*-* 12:00:00"
    systemd-analyze calendar "quarterly"
    systemd-analyze calendar "semi-annually"
    systemd-analyze calendar "semiannually"
    AEEOF
    chmod +x TEST-74-AUX-UTILS.analyze-edge.sh
  '';
}

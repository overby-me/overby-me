{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.analyze\\-standalone\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.analyze-standalone.sh << 'ANEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "systemd-analyze calendar parses calendar specs"
    systemd-analyze calendar "daily"
    systemd-analyze calendar "*-*-* 00:00:00"
    systemd-analyze calendar "Mon *-*-* 12:00:00"

    : "systemd-analyze calendar --iterations shows next N occurrences"
    systemd-analyze calendar --iterations=3 "hourly"

    : "systemd-analyze timespan parses time spans"
    systemd-analyze timespan "1h 30min"
    systemd-analyze timespan "2days"
    systemd-analyze timespan "500ms"

    : "systemd-analyze timestamp parses timestamps"
    systemd-analyze timestamp "now"
    systemd-analyze timestamp "today"
    systemd-analyze timestamp "yesterday"

    : "systemd-analyze unit-paths shows search paths"
    systemd-analyze unit-paths

    : "Invalid inputs return errors"
    (! systemd-analyze calendar "not-a-valid-spec-at-all")
    (! systemd-analyze timespan "not-a-timespan")
    ANEOF
    chmod +x TEST-74-AUX-UTILS.analyze-standalone.sh
  '';
}

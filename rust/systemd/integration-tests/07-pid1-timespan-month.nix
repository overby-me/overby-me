{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.timespan-month\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.timespan-month.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    SLEEP="$(command -v sleep)"

    at_exit() {
        set +e
        systemctl stop tsm-cap.service tsm-min.service tsm-word.service 2>/dev/null
        rm -f /run/systemd/system/tsm-cap.service /run/systemd/system/tsm-min.service /run/systemd/system/tsm-word.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    mkunit() {
        cat > "/run/systemd/system/$1.service" << EOF
    [Service]
    Type=simple
    RuntimeMaxSec=$2
    ExecStart=$SLEEP infinity
    EOF
    }

    # `M` = month, `m`/`min` = minute (systemd's time units are case-sensitive);
    # a month is C's 30.44-day USEC_PER_MONTH = 2629800000000 us, not 30 days.
    mkunit tsm-cap 1M
    mkunit tsm-min 1min
    mkunit tsm-word 1month
    systemctl daemon-reload
    systemctl start tsm-cap.service tsm-min.service tsm-word.service

    # Strip any trailing us unit the property renderer appends, comparing the
    # raw microsecond value.
    rmusec() { systemctl show -P RuntimeMaxUSec "$1" | sed 's/us$//'; }

    : "RuntimeMaxSec=1M is one month (2629800000000 us), not one minute"
    test "$(rmusec tsm-cap.service)" = "2629800000000"
    : "RuntimeMaxSec=1min stays one minute (proves M != m)"
    test "$(rmusec tsm-min.service)" = "60000000"
    : "RuntimeMaxSec=1month matches C's 30.44-day month"
    test "$(rmusec tsm-word.service)" = "2629800000000"
    RIDEOF
  '';
}

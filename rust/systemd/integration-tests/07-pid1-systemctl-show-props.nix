{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.systemctl-show-props\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.systemctl-show-props.sh << 'SPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    at_exit() {
        set +e
        systemctl stop show-props-test.service 2>/dev/null
        rm -f /run/systemd/system/show-props-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "systemctl show with multiple -p flags"
    cat > /run/systemd/system/show-props-test.service << EOF
    [Unit]
    Description=Show props test
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start show-props-test.service
    systemctl is-active show-props-test.service
    # Show multiple properties
    OUT="$(systemctl show -P ActiveState -P SubState -P Type show-props-test.service)"
    echo "$OUT" | grep -q "active"
    echo "$OUT" | grep -q "oneshot"
    # Show with -p (key=value format)
    systemctl show -p ActiveState -p Type show-props-test.service | grep -q "ActiveState=active"
    systemctl show -p ActiveState -p Type show-props-test.service | grep -q "Type=oneshot"
    systemctl stop show-props-test.service
    SPEOF
  '';
}

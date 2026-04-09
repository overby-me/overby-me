{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.systemctl-show\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.systemctl-show.sh << 'SSEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop show-test.service 2>/dev/null
        rm -f /run/systemd/system/show-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "systemctl show -P returns property values"
    cat > /run/systemd/system/show-test.service << EOF
    [Unit]
    Description=Show test service
    [Service]
    ExecStart=sleep infinity
    EOF
    retry systemctl daemon-reload
    [[ "$(systemctl show -P Description show-test.service)" == "Show test service" ]]

    : "systemctl show -P ActiveState before/after start"
    [[ "$(systemctl show -P ActiveState show-test.service)" == "inactive" ]]
    retry systemctl start show-test.service
    [[ "$(systemctl show -P ActiveState show-test.service)" == "active" ]]
    systemctl stop show-test.service
    [[ "$(systemctl show -P ActiveState show-test.service)" == "inactive" ]]
    SSEOF
  '';
}

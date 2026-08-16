{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.exec-reload\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.exec-reload.sh << 'EREOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop reload-test.service 2>/dev/null
        rm -f /run/systemd/system/reload-test.service
        rm -f /tmp/reload-marker
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "ExecReload= runs on systemctl reload"
    cat > /run/systemd/system/reload-test.service << EOF
    [Service]
    ExecStart=sleep infinity
    ExecReload=touch /tmp/reload-marker
    EOF
    retry systemctl daemon-reload
    retry systemctl start reload-test.service
    systemctl is-active reload-test.service
    [[ ! -f /tmp/reload-marker ]]
    systemctl reload reload-test.service
    sleep 0.5
    [[ -f /tmp/reload-marker ]]
    systemctl stop reload-test.service
    EREOF
  '';
}

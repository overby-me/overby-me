{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.is-enabled\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.is-enabled.sh << 'IEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl disable is-enabled-test.service 2>/dev/null
        systemctl unmask is-enabled-test.service 2>/dev/null
        rm -f /run/systemd/system/is-enabled-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "systemctl is-enabled for disabled service"
    cat > /run/systemd/system/is-enabled-test.service << EOF
    [Service]
    Type=oneshot
    ExecStart=true
    [Install]
    WantedBy=multi-user.target
    EOF
    retry systemctl daemon-reload
    # Should not be enabled yet
    [[ "$(systemctl is-enabled is-enabled-test.service)" == "disabled" ]]

    : "systemctl is-enabled after enable"
    systemctl enable is-enabled-test.service
    [[ "$(systemctl is-enabled is-enabled-test.service)" == "enabled" ]]

    : "systemctl is-enabled after disable"
    systemctl disable is-enabled-test.service
    [[ "$(systemctl is-enabled is-enabled-test.service)" == "disabled" ]]

    : "systemctl is-enabled for masked service"
    systemctl mask is-enabled-test.service
    [[ "$(systemctl is-enabled is-enabled-test.service)" == "masked" ]]
    systemctl unmask is-enabled-test.service
    IEEOF
  '';
}

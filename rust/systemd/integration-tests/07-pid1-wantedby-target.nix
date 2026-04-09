{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.wantedby-target\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.wantedby-target.sh << 'WTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl disable wantedby-test.service 2>/dev/null
        systemctl stop wantedby-test.service custom-test.target 2>/dev/null
        rm -f /run/systemd/system/wantedby-test.service
        rm -f /run/systemd/system/custom-test.target
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "WantedBy= creates symlink on enable and target starts service"
    cat > /run/systemd/system/custom-test.target << EOF
    [Unit]
    Description=Custom test target
    EOF
    cat > /run/systemd/system/wantedby-test.service << EOF
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    [Install]
    WantedBy=custom-test.target
    EOF
    retry systemctl daemon-reload
    systemctl enable wantedby-test.service
    # Verify symlink was created
    [[ -L /etc/systemd/system/custom-test.target.wants/wantedby-test.service ]]
    # Starting the target should pull in the service
    systemctl start custom-test.target
    systemctl is-active wantedby-test.service
    systemctl stop custom-test.target wantedby-test.service
    systemctl disable wantedby-test.service
    # Verify symlink was removed
    [[ ! -L /etc/systemd/system/custom-test.target.wants/wantedby-test.service ]]
    WTEOF
  '';
}

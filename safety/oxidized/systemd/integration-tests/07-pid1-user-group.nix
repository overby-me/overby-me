{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.user-group\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.user-group.sh << 'UGEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/user-group-test-*.service
        rm -f /tmp/user-group-*
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "User= runs process as specified user"
    cat > /run/systemd/system/user-group-test-user.service << EOF
    [Service]
    Type=oneshot
    User=testuser
    ExecStart=bash -c 'id -nu > /tmp/user-group-user'
    EOF
    retry systemctl daemon-reload
    retry systemctl start user-group-test-user.service
    [[ "$(cat /tmp/user-group-user)" == "testuser" ]]

    : "Group= runs process with specified group"
    cat > /run/systemd/system/user-group-test-group.service << EOF
    [Service]
    Type=oneshot
    User=testuser
    Group=daemon
    ExecStart=bash -c 'id -ng > /tmp/user-group-group'
    EOF
    systemctl daemon-reload
    systemctl start user-group-test-group.service
    [[ "$(cat /tmp/user-group-group)" == "daemon" ]]

    : "SupplementaryGroups= adds extra groups"
    cat > /run/systemd/system/user-group-test-suppl.service << EOF
    [Service]
    Type=oneshot
    User=testuser
    SupplementaryGroups=daemon
    ExecStart=bash -c 'id -Gn > /tmp/user-group-suppl'
    EOF
    systemctl daemon-reload
    systemctl start user-group-test-suppl.service
    grep -q "daemon" /tmp/user-group-suppl
    UGEOF
  '';
}

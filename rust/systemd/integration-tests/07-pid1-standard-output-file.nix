{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.standard-output-file\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.standard-output-file.sh << 'SOEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/stdout-test.service
        rm -f /tmp/stdout-test-out /tmp/stdout-test-err
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "StandardOutput=file: writes stdout to file"
    cat > /run/systemd/system/stdout-test.service << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'echo hello-stdout'
    StandardOutput=file:/tmp/stdout-test-out
    StandardError=file:/tmp/stdout-test-err
    EOF
    retry systemctl daemon-reload
    retry systemctl start stdout-test.service
    [[ "$(cat /tmp/stdout-test-out)" == "hello-stdout" ]]

    : "StandardOutput=append: appends to file"
    # Stop previous oneshot so re-start actually runs again
    systemctl stop stdout-test.service 2>/dev/null || true
    cat > /run/systemd/system/stdout-test.service << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'echo second-line'
    StandardOutput=append:/tmp/stdout-test-out
    EOF
    retry systemctl daemon-reload
    retry systemctl start stdout-test.service
    # Should have both lines
    grep -q "hello-stdout" /tmp/stdout-test-out
    grep -q "second-line" /tmp/stdout-test-out

    : "StandardOutput=truncate: overwrites file"
    systemctl stop stdout-test.service 2>/dev/null || true
    cat > /run/systemd/system/stdout-test.service << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'echo only-this'
    StandardOutput=truncate:/tmp/stdout-test-out
    EOF
    retry systemctl daemon-reload
    retry systemctl start stdout-test.service
    [[ "$(cat /tmp/stdout-test-out)" == "only-this" ]]
    SOEOF
  '';
}

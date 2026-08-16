{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.requires-mounts-for\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.requires-mounts-for.sh << 'RMEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/rmf-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "RequiresMountsFor= ensures mount points are available"
    cat > /run/systemd/system/rmf-test.service << EOF
    [Unit]
    RequiresMountsFor=/tmp
    [Service]
    Type=oneshot
    ExecStart=bash -c 'mountpoint / && test -d /tmp'
    EOF
    retry systemctl daemon-reload
    retry systemctl start rmf-test.service
    RMEOF
  '';
}

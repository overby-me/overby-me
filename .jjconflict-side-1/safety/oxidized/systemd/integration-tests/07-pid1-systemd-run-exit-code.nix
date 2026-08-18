{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.systemd-run-exit-code\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.systemd-run-exit-code.sh << 'SREOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "systemd-run --wait forwards exit code 0"
    systemd-run --wait --pipe true

    : "systemd-run --wait forwards nonzero exit code"
    RC=0
    systemd-run --wait --pipe bash -c 'exit 42' || RC=$?
    [[ "$RC" -eq 42 ]]

    : "systemd-run --wait with -p Type=oneshot"
    systemd-run --wait -p Type=oneshot true
    SREOF
  '';
}

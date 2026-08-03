{
  name = "07-PID1";
  # Boot PID 1 with SYSTEMD_RS_INIT_SCOPE=1 so it applies init.scope.d resource
  # controls to its own cgroup (task #12 slice A).
  initScope = true;
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.init-scope\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.init-scope.sh << 'ISEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/init.scope.d/50-test.conf
        systemctl daemon-reload
    }
    trap at_exit EXIT

    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "init.scope.d MemoryMax= is applied to PID 1's init.scope cgroup"
    mkdir -p /run/systemd/system/init.scope.d
    cat > /run/systemd/system/init.scope.d/50-test.conf << EOF
    [Scope]
    MemoryMax=536870912
    EOF
    retry systemctl daemon-reload

    mm=$(cat /sys/fs/cgroup/init.scope/memory.max)
    echo "init.scope memory.max = $mm (want 536870912)"
    test "$mm" = "536870912"
    ISEOF
  '';
}

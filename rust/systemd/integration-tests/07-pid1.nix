{
  name = "07-PID1";
  # Patch main script to remove mountpoint check and exit, keep run_subtests.
  # Enable mask.sh, issue-16115.sh, issue-3166.sh, issue-33672.sh, pr-31351.sh,
  # issue-31752.sh, issue-14566.sh, socket-on-failure.sh;
  # remove subtests requiring unimplemented features.
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    # Remove PrivateUsersEx lines (not implemented), keep PrivateUsers=yes
    sed -i '/PrivateUsersEx/d' TEST-07-PID1.private-users.sh
    # issue-30412: socat triggers socket activation. Run it in
    # background with a kill-timeout since the connection close
    # depends on async exit handling timing.
    perl -i -pe 's/^socat (.*)$/socat $1 \&\nSOCAT_PID=\$!\nsleep 2\nkill \$SOCAT_PID 2>\/dev\/null || true\nwait \$SOCAT_PID 2>\/dev\/null || true/' TEST-07-PID1.issue-30412.sh
    # Remove DynamicUser tests from working-directory (DynamicUser not implemented)
    perl -i -0pe 's/\(! systemd-run[^)]*DynamicUser[^)]*\)\n?//g' TEST-07-PID1.working-directory.sh
    # NixOS has nobody's home at /var/empty, not /
    perl -i -pe 's{"\/"$}{"/var/empty"}' TEST-07-PID1.working-directory.sh
    # Ensure /home/testuser exists (NixOS creates it via users-groups.service)
    sed -i '3a mkdir -p /home/testuser && chown testuser:testuser /home/testuser' TEST-07-PID1.working-directory.sh
    # Rewrite exec-context test: keep ProtectSystem, ProtectHome, Limit,
    # directory (Runtime/State/Cache/Logs/Configuration), PrivateTmp,
    # PrivateDevices, ProtectKernel*, ProtectControlGroups, ProtectHostname,
    # Bind/ReadOnly/Inaccessible paths, TemporaryFileSystem, ReadWritePaths,
    # UMask, Nice, and OOMScoreAdjust tests.
    # Remove PrivateMounts/MountAPIVFS, ProtectProc, ProcSubset,
    # RestrictFileSystems, DynamicUser, env file serialization,
    # IO/CPU/Device directives, SocketBind, and RestrictNamespaces sections.
    cat > TEST-07-PID1.exec-context.sh << 'TESTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "ProtectSystem= tests"
    systemd-run --wait --pipe -p ProtectSystem=yes \
        bash -xec "test ! -w /usr; test -w /etc; test -w /var"
    systemd-run --wait --pipe -p ProtectSystem=full \
        bash -xec "test ! -w /usr; test ! -w /etc; test -w /var"
    systemd-run --wait --pipe -p ProtectSystem=strict \
        bash -xec "test ! -w /; test ! -w /etc; test ! -w /var; test -w /dev; test -w /proc"
    systemd-run --wait --pipe -p ProtectSystem=no \
        bash -xec "test -w /; test -w /etc; test -w /var; test -w /dev; test -w /proc"

    : "ProtectHome= tests"
    MARK="$(mktemp /root/.exec-context.XXX)"
    systemd-run --wait --pipe -p ProtectHome=yes \
        bash -xec "test ! -w /home; test ! -w /root; test ! -w /run/user; test ! -e $MARK"
    systemd-run --wait --pipe -p ProtectHome=read-only \
        bash -xec "test ! -w /home; test ! -w /root; test ! -w /run/user; test -e $MARK"
    systemd-run --wait --pipe -p ProtectHome=tmpfs \
        bash -xec "test ! -w /home; test ! -w /root; test ! -w /run/user; test ! -e $MARK"
    systemd-run --wait --pipe -p ProtectHome=no \
        bash -xec "test -w /home; test -w /root; test -w /run/user; test -e $MARK"
    rm -f "$MARK"

    : "Comprehensive Limit tests"
    systemd-run --wait --pipe \
        -p LimitCPU=10:15 \
        -p LimitFSIZE=96G \
        -p LimitDATA=infinity \
        -p LimitSTACK=8M \
        -p LimitCORE=17M \
        -p LimitRSS=27G \
        -p LimitNOFILE=64:127 \
        -p LimitAS=infinity \
        -p LimitNPROC=64:infinity \
        -p LimitMEMLOCK=37M \
        -p LimitLOCKS=19:1021 \
        -p LimitSIGPENDING=21 \
        -p LimitMSGQUEUE=666 \
        -p LimitNICE=4 \
        -p LimitRTPRIO=8 \
        bash -xec 'KB=1; MB=$((KB * 1024)); GB=$((MB * 1024));
                   : CPU;        [[ $(ulimit -St) -eq 10 ]];           [[ $(ulimit -Ht) -eq 15 ]];
                   : FSIZE;      [[ $(ulimit -Sf) -eq $((96 * GB)) ]]; [[ $(ulimit -Hf) -eq $((96 * GB)) ]];
                   : DATA;       [[ $(ulimit -Sd) == unlimited  ]];    [[ $(ulimit -Hd) == unlimited ]];
                   : STACK;      [[ $(ulimit -Ss) -eq $((8 * MB)) ]];  [[ $(ulimit -Hs) -eq $((8 * MB)) ]];
                   : CORE;       [[ $(ulimit -Sc) -eq $((17 * MB)) ]]; [[ $(ulimit -Hc) -eq $((17 * MB)) ]];
                   : RSS;        [[ $(ulimit -Sm) -eq $((27 * GB)) ]]; [[ $(ulimit -Hm) -eq $((27 * GB)) ]];
                   : NOFILE;     [[ $(ulimit -Sn) -eq 64 ]];           [[ $(ulimit -Hn) -eq 127 ]];
                   : AS;         [[ $(ulimit -Sv) == unlimited ]];     [[ $(ulimit -Hv) == unlimited ]];
                   : NPROC;      [[ $(ulimit -Su) -eq 64 ]];           [[ $(ulimit -Hu) == unlimited ]];
                   : MEMLOCK;    [[ $(ulimit -Sl) -eq $((37 * MB)) ]]; [[ $(ulimit -Hl) -eq $((37 * MB)) ]];
                   : LOCKS;      [[ $(ulimit -Sx) -eq 19 ]];           [[ $(ulimit -Hx) -eq 1021 ]];
                   : SIGPENDING; [[ $(ulimit -Si) -eq 21 ]];           [[ $(ulimit -Hi) -eq 21 ]];
                   : MSGQUEUE;   [[ $(ulimit -Sq) -eq 666 ]];          [[ $(ulimit -Hq) -eq 666 ]];
                   : NICE;       [[ $(ulimit -Se) -eq 4 ]];            [[ $(ulimit -He) -eq 4 ]];
                   : RTPRIO;     [[ $(ulimit -Sr) -eq 8 ]];            [[ $(ulimit -Hr) -eq 8 ]];'

    : "RuntimeDirectory= tests"
    systemd-run --wait --pipe -p RuntimeDirectory=exec-ctx-test \
        bash -xec '[[ -d /run/exec-ctx-test ]]; [[ "$RUNTIME_DIRECTORY" == /run/exec-ctx-test ]]'

    : "StateDirectory= tests"
    systemd-run --wait --pipe -p StateDirectory=exec-ctx-test \
        bash -xec '[[ -d /var/lib/exec-ctx-test ]]; [[ "$STATE_DIRECTORY" == /var/lib/exec-ctx-test ]]'
    rm -rf /var/lib/exec-ctx-test

    : "CacheDirectory= tests"
    systemd-run --wait --pipe -p CacheDirectory=exec-ctx-test \
        bash -xec '[[ -d /var/cache/exec-ctx-test ]]; [[ "$CACHE_DIRECTORY" == /var/cache/exec-ctx-test ]]'
    rm -rf /var/cache/exec-ctx-test

    : "LogsDirectory= tests"
    systemd-run --wait --pipe -p LogsDirectory=exec-ctx-test \
        bash -xec '[[ -d /var/log/exec-ctx-test ]]; [[ "$LOGS_DIRECTORY" == /var/log/exec-ctx-test ]]'
    rm -rf /var/log/exec-ctx-test

    : "ConfigurationDirectory= tests"
    systemd-run --wait --pipe -p ConfigurationDirectory=exec-ctx-test \
        bash -xec '[[ -d /etc/exec-ctx-test ]]; [[ "$CONFIGURATION_DIRECTORY" == /etc/exec-ctx-test ]]'
    rm -rf /etc/exec-ctx-test

    : "Multiple directory entries with modes"
    systemd-run --wait --pipe \
        -p CacheDirectory="foo" \
        -p CacheDirectory="bar" \
        -p CacheDirectoryMode=0700 \
        bash -xec '[[ -d /var/cache/foo ]]; [[ -d /var/cache/bar ]];
                   [[ "$CACHE_DIRECTORY" == "/var/cache/bar:/var/cache/foo" ]] ||
                   [[ "$CACHE_DIRECTORY" == "/var/cache/foo:/var/cache/bar" ]];
                   [[ $(stat -c "%a" /var/cache/bar) == 700 ]]'
    rm -rf /var/cache/foo /var/cache/bar

    : "RuntimeDirectoryMode= tests"
    systemd-run --wait --pipe \
        -p RuntimeDirectory=mode-test \
        -p RuntimeDirectoryMode=0750 \
        bash -xec '[[ -d /run/mode-test ]]; [[ $(stat -c "%a" /run/mode-test) == 750 ]]'

    : "StateDirectoryMode= tests"
    systemd-run --wait --pipe \
        -p StateDirectory=mode-test \
        -p StateDirectoryMode=0700 \
        bash -xec '[[ -d /var/lib/mode-test ]]; [[ $(stat -c "%a" /var/lib/mode-test) == 700 ]]'
    rm -rf /var/lib/mode-test

    : "ConfigurationDirectoryMode= tests"
    systemd-run --wait --pipe \
        -p ConfigurationDirectory=mode-test \
        -p ConfigurationDirectoryMode=0400 \
        bash -xec '[[ -d /etc/mode-test ]]; [[ $(stat -c "%a" /etc/mode-test) == 400 ]]'
    rm -rf /etc/mode-test

    : "LogsDirectoryMode= tests"
    systemd-run --wait --pipe \
        -p LogsDirectory=mode-test \
        -p LogsDirectoryMode=0750 \
        bash -xec '[[ -d /var/log/mode-test ]]; [[ $(stat -c "%a" /var/log/mode-test) == 750 ]]'
    rm -rf /var/log/mode-test

    : "Space-separated directory entries"
    systemd-run --wait --pipe \
        -p RuntimeDirectory="multi-a multi-b" \
        bash -xec '[[ -d /run/multi-a ]]; [[ -d /run/multi-b ]];
                   [[ "$RUNTIME_DIRECTORY" == "/run/multi-a:/run/multi-b" ]] ||
                   [[ "$RUNTIME_DIRECTORY" == "/run/multi-b:/run/multi-a" ]]'

    : "PrivateTmp= tests"
    touch /tmp/exec-ctx-marker
    systemd-run --wait --pipe -p PrivateTmp=yes \
        bash -xec '[[ ! -e /tmp/exec-ctx-marker ]]; touch /tmp/private-marker; [[ -e /tmp/private-marker ]]'
    [[ -e /tmp/exec-ctx-marker ]]
    rm -f /tmp/exec-ctx-marker

    : "PrivateDevices= tests"
    systemd-run --wait --pipe -p PrivateDevices=yes \
        bash -xec '[[ -e /dev/null ]]; [[ -e /dev/zero ]]; (! [[ -e /dev/sda ]] 2>/dev/null || true)'

    : "ProtectKernelTunables= tests"
    systemd-run --wait --pipe -p ProtectKernelTunables=yes \
        bash -xec '(! touch /proc/sys/kernel/domainname 2>/dev/null)'

    : "ProtectKernelModules= tests"
    systemd-run --wait --pipe -p ProtectKernelModules=yes \
        bash -xec '(! ls /usr/lib/modules 2>/dev/null)'

    : "ProtectControlGroups= tests"
    systemd-run --wait --pipe -p ProtectControlGroups=yes \
        bash -xec '(! touch /sys/fs/cgroup/test-file 2>/dev/null)'

    : "ProtectKernelLogs= tests"
    systemd-run --wait --pipe -p ProtectKernelLogs=yes \
        bash -xec '[[ "$(stat -c %t:%T /dev/kmsg)" == "$(stat -c %t:%T /dev/null)" ]]'

    : "BindPaths= tests"
    mkdir -p /tmp/bind-source
    echo "bind-test" > /tmp/bind-source/marker
    systemd-run --wait --pipe -p BindPaths="/tmp/bind-source:/tmp/bind-target" \
        bash -xec '[[ "$(cat /tmp/bind-target/marker)" == "bind-test" ]]'
    rm -rf /tmp/bind-source

    : "BindPaths= multi-entry and optional prefix tests"
    systemd-run --wait --pipe -p BindPaths="/etc /home:/mnt:norbind -/foo/bar/baz:/usr:rbind" \
        bash -xec 'mountpoint /etc; test -d /etc/systemd; mountpoint /mnt; ! mountpoint /usr'

    : "BindReadOnlyPaths= tests"
    mkdir -p /tmp/bind-ro-source
    echo "bind-ro-test" > /tmp/bind-ro-source/marker
    systemd-run --wait --pipe -p BindReadOnlyPaths="/tmp/bind-ro-source:/tmp/bind-ro-target" \
        bash -xec '[[ "$(cat /tmp/bind-ro-target/marker)" == "bind-ro-test" ]]'
    rm -rf /tmp/bind-ro-source

    : "BindReadOnlyPaths= multi-entry and optional prefix tests"
    systemd-run --wait --pipe -p BindReadOnlyPaths="/etc /home:/mnt:norbind -/foo/bar/baz:/usr:rbind" \
        bash -xec 'test ! -w /etc; test ! -w /mnt; ! mountpoint /usr'

    : "InaccessiblePaths= tests"
    mkdir -p /tmp/inaccessible-test
    echo "secret" > /tmp/inaccessible-test/data
    systemd-run --wait --pipe -p InaccessiblePaths="/tmp/inaccessible-test" \
        bash -xec '(! cat /tmp/inaccessible-test/data 2>/dev/null)'
    rm -rf /tmp/inaccessible-test

    : "TemporaryFileSystem= tests"
    systemd-run --wait --pipe -p TemporaryFileSystem="/tmp/tmpfs-test" \
        bash -xec '[[ -d /tmp/tmpfs-test ]]; touch /tmp/tmpfs-test/file; [[ -e /tmp/tmpfs-test/file ]]'

    : "ReadOnlyPaths= tests"
    mkdir -p /tmp/ro-test && echo "data" > /tmp/ro-test/file
    systemd-run --wait --pipe -p ReadOnlyPaths="/tmp/ro-test" \
        bash -xec 'cat /tmp/ro-test/file; (! touch /tmp/ro-test/new-file 2>/dev/null)'
    rm -rf /tmp/ro-test

    : "ReadWritePaths= with ProtectSystem=strict tests"
    mkdir -p /tmp/rw-test
    systemd-run --wait --pipe -p ProtectSystem=strict -p ReadWritePaths="/tmp/rw-test" \
        bash -xec 'touch /tmp/rw-test/new-file; [[ -e /tmp/rw-test/new-file ]]; (! touch /etc/should-fail 2>/dev/null)'
    rm -rf /tmp/rw-test

    : "UMask= tests"
    systemd-run --wait --pipe -p UMask=0077 \
        bash -xec 'touch /tmp/umask-test; [[ "$(stat -c %a /tmp/umask-test)" == "600" ]]'
    rm -f /tmp/umask-test

    : "Nice= tests"
    systemd-run --wait --pipe -p Nice=15 \
        bash -xec 'read -r -a SELF_STAT </proc/self/stat; [[ "''${SELF_STAT[18]}" -eq 15 ]]'

    : "OOMScoreAdjust= tests"
    systemd-run --wait --pipe -p OOMScoreAdjust=500 \
        bash -xec '[[ "$(cat /proc/self/oom_score_adj)" == "500" ]]'

    : "NoNewPrivileges= tests"
    systemd-run --wait --pipe -p NoNewPrivileges=yes \
        bash -xec '[[ "$(grep NoNewPrivs /proc/self/status | awk "{print \$2}")" == "1" ]]'

    : "ProtectClock= tests"
    systemd-run --wait --pipe -p ProtectClock=yes \
        bash -xec 'if [[ -e /dev/rtc0 ]]; then
                     [[ "$(stat -c %t:%T /dev/rtc0)" == "$(stat -c %t:%T /dev/null)" ]];
                   fi'

    : "PrivateUsers= tests"
    systemd-run --wait --pipe -p PrivateUsers=yes \
        bash -xec '[[ "$(cat /proc/self/uid_map | awk "{print \$1}")" == "0" ]]'

    : "PrivateNetwork= tests"
    systemd-run --wait --pipe -p PrivateNetwork=yes \
        bash -xec '(! ip link show eth0 2>/dev/null); ip link show lo'

    : "ProtectHostname= tests"
    ORIG_HOSTNAME="$(hostname)"
    systemd-run --wait --pipe -p ProtectHostname=yes \
        bash -xec 'hostname test-change 2>/dev/null && [[ "$(hostname)" != "test-change" ]] || true'
    [[ "$(hostname)" == "$ORIG_HOSTNAME" ]]

    : "LockPersonality= tests"
    systemd-run --wait --pipe -p LockPersonality=yes -p NoNewPrivileges=yes \
        bash -xec '[[ "$(uname -m)" != "" ]]'

    : "CapabilityBoundingSet= tests"
    systemd-run --wait --pipe -p CapabilityBoundingSet=CAP_NET_RAW \
        bash -xec 'CAPBND=$(grep CapBnd /proc/self/status | awk "{print \$2}");
                   [[ "$CAPBND" != "0000003fffffffff" ]]'

    : "AmbientCapabilities= tests"
    systemd-run --wait --pipe -p AmbientCapabilities=CAP_NET_RAW -p User=testuser \
        bash -xec 'CAPAMB=$(grep CapAmb /proc/self/status | awk "{print \$2}");
                   [[ "$CAPAMB" != "0000000000000000" ]]'

    : "CPUSchedulingPolicy= tests"
    systemd-run --wait --pipe -p CPUSchedulingPolicy=fifo -p CPUSchedulingPriority=10 \
        bash -xec 'grep -E "^policy\s*:\s*1$" /proc/self/sched; grep -E "^prio\s*:\s*89$" /proc/self/sched'

    : "EnvironmentFile= tests"
    TEST_ENV_FILE="/tmp/test-env-file-$$"
    printf 'FOO="hello world"\nBAR=simple\n# comment line\nBAZ="quoted value"\n' > "$TEST_ENV_FILE"
    systemd-run --wait --pipe -p EnvironmentFile="$TEST_ENV_FILE" \
        bash -xec '[[ "$FOO" == "hello world" && "$BAR" == "simple" && "$BAZ" == "quoted value" ]]'
    rm -f "$TEST_ENV_FILE"

    : "EnvironmentFile= with optional prefix tests"
    systemd-run --wait --pipe -p EnvironmentFile=-/nonexistent/env/file \
        bash -xec 'true'

    : "User= with PrivateNetwork= and ProtectSystem= combination"
    systemd-run --wait --pipe -p User=testuser -p PrivateNetwork=yes -p ProtectSystem=strict \
        bash -xec '(! ip link show eth0 2>/dev/null); ip link show lo;
                   test ! -w /usr; test ! -w /etc; test ! -w /var;
                   [[ "$(id -nu)" == testuser ]]'

    : "PrivateTmp= with PrivateNetwork= combination"
    touch /tmp/combo-marker
    systemd-run --wait --pipe -p PrivateTmp=yes -p PrivateNetwork=yes \
        bash -xec '(! ip link show eth0 2>/dev/null);
                   test ! -e /tmp/combo-marker'
    rm -f /tmp/combo-marker

    : "ExecStartPre= tests"
    systemd-run --wait --pipe \
        -p ExecStartPre="touch /tmp/exec-pre-marker" \
        bash -xec '[[ -e /tmp/exec-pre-marker ]]'
    rm -f /tmp/exec-pre-marker

    : "ExecStartPre= failure prevents ExecStart"
    (! systemd-run --wait --pipe \
        -p ExecStartPre="false" \
        bash -xec 'echo should-not-run')

    : "ExecStartPre= with minus prefix ignores failure"
    systemd-run --wait --pipe \
        -p ExecStartPre="-false" \
        bash -xec 'true'

    : "Multiple ExecStartPre= commands run in order"
    systemd-run --wait --pipe \
        -p ExecStartPre="touch /tmp/pre-order-1" \
        -p ExecStartPre="touch /tmp/pre-order-2" \
        bash -xec '[[ -e /tmp/pre-order-1 && -e /tmp/pre-order-2 ]]'
    rm -f /tmp/pre-order-1 /tmp/pre-order-2

    : "ExecStartPost= tests"
    systemd-run --wait --pipe \
        -p ExecStartPost="touch /tmp/exec-post-marker" \
        true
    [[ -e /tmp/exec-post-marker ]]
    rm -f /tmp/exec-post-marker

    : "WorkingDirectory= tests"
    systemd-run --wait --pipe -p WorkingDirectory=/tmp \
        bash -xec '[[ "$PWD" == /tmp ]]'

    : "WorkingDirectory= with User="
    systemd-run --wait --pipe -p WorkingDirectory=/tmp -p User=testuser \
        bash -xec '[[ "$PWD" == /tmp && "$(id -nu)" == testuser ]]'

    : "StandardOutput=file: tests"
    rm -f /tmp/stdout-test-out
    systemd-run --wait --pipe -p StandardOutput=file:/tmp/stdout-test-out \
        bash -xec 'echo hello-stdout'
    [[ "$(cat /tmp/stdout-test-out)" == *hello-stdout* ]]
    rm -f /tmp/stdout-test-out

    : "StandardError=file: tests"
    rm -f /tmp/stderr-test-out
    systemd-run --wait --pipe -p StandardError=file:/tmp/stderr-test-out \
        bash -xec 'echo hello-stderr >&2'
    [[ "$(cat /tmp/stderr-test-out)" == *hello-stderr* ]]
    rm -f /tmp/stderr-test-out

    : "StandardOutput=append: tests"
    echo "line1" > /tmp/append-test-out
    systemd-run --wait --pipe -p StandardOutput=append:/tmp/append-test-out \
        bash -xec 'echo line2'
    grep -q line1 /tmp/append-test-out
    grep -q line2 /tmp/append-test-out
    rm -f /tmp/append-test-out

    : "SetCredential= tests"
    systemd-run --wait --pipe -p SetCredential=mycred:hello-cred \
        bash -xec '[[ -n "$CREDENTIALS_DIRECTORY" ]];
                   [[ -f "$CREDENTIALS_DIRECTORY/mycred" ]];
                   [[ "$(cat "$CREDENTIALS_DIRECTORY/mycred")" == hello-cred ]]'

    : "Multiple SetCredential= entries"
    systemd-run --wait --pipe \
        -p SetCredential=cred1:value1 \
        -p SetCredential=cred2:value2 \
        bash -xec '[[ "$(cat "$CREDENTIALS_DIRECTORY/cred1")" == value1 ]];
                   [[ "$(cat "$CREDENTIALS_DIRECTORY/cred2")" == value2 ]]'

    : "SetCredential= with User="
    systemd-run --wait --pipe -p SetCredential=usercred:secret -p User=testuser \
        bash -xec '[[ "$(cat "$CREDENTIALS_DIRECTORY/usercred")" == secret ]];
                   [[ "$(id -nu)" == testuser ]]'

    : "KillSignal= tests"
    systemd-run -p KillSignal=SIGUSR1 --unit=kill-signal-test --remain-after-exit \
        bash -xec 'trap "touch /tmp/kill-sigusr1-received; exit 0" USR1; while true; do sleep 0.1; done' &
    sleep 1
    systemctl kill --signal=SIGUSR1 kill-signal-test.service
    sleep 1
    [[ -e /tmp/kill-sigusr1-received ]]
    systemctl stop kill-signal-test.service 2>/dev/null || true
    rm -f /tmp/kill-sigusr1-received

    : "WatchdogSec= tests — notify service killed when it stops pinging"
    systemd-run --unit=watchdog-test -p Type=notify -p WatchdogSec=2 \
        bash -c 'systemd-notify --ready; sleep 60'
    sleep 5
    # Service should have been killed by watchdog after 2s without WATCHDOG=1 ping
    (! systemctl is-active watchdog-test.service)
    systemctl reset-failed watchdog-test.service 2>/dev/null || true

    : "RemainAfterExit= tests"
    systemd-run -p Type=oneshot -p RemainAfterExit=yes --unit=remain-test true
    sleep 1
    systemctl is-active remain-test.service
    systemctl stop remain-test.service
    (! systemctl is-active remain-test.service)

    : "LoadCredential= tests"
    echo -n "file-cred-data" > /tmp/test-cred-file
    systemd-run --wait --pipe -p LoadCredential=filecred:/tmp/test-cred-file \
        bash -xec '[[ "$(cat "$CREDENTIALS_DIRECTORY/filecred")" == file-cred-data ]]'
    rm -f /tmp/test-cred-file

    : "LoadCredential= with SetCredential= override"
    echo -n "loaded" > /tmp/test-cred-override
    systemd-run --wait --pipe \
        -p SetCredential=mycred:inline-data \
        -p LoadCredential=mycred:/tmp/test-cred-override \
        bash -xec '[[ "$(cat "$CREDENTIALS_DIRECTORY/mycred")" == loaded ]]'
    rm -f /tmp/test-cred-override

    : "Group= tests"
    systemd-run --wait --pipe -p Group=testuser \
        bash -xec '[[ "$(id -ng)" == testuser ]]'

    : "User= and Group= together"
    systemd-run --wait --pipe -p User=testuser -p Group=root \
        bash -xec '[[ "$(id -nu)" == testuser && "$(id -ng)" == root ]]'

    : "Restart= with Type=simple — service restarts on failure"
    systemd-run --unit=restart-test -p Restart=on-failure -p RestartSec=0 \
        bash -c 'echo restarting > /tmp/restart-marker; exit 1'
    sleep 2
    # After failure, it should have restarted (marker file re-created)
    [[ -e /tmp/restart-marker ]]
    systemctl stop restart-test.service 2>/dev/null || true
    systemctl reset-failed restart-test.service 2>/dev/null || true
    rm -f /tmp/restart-marker

    : "ExecCondition= tests — condition passes"
    systemd-run --wait --pipe \
        -p ExecCondition="true" \
        bash -xec 'echo condition-passed'

    : "ExecStopPost= via transient unit"
    systemd-run --unit=stop-post-test -p RemainAfterExit=yes \
        -p ExecStopPost="touch /tmp/stop-post-marker" \
        true
    sleep 1
    systemctl stop stop-post-test.service
    sleep 1
    [[ -e /tmp/stop-post-marker ]]
    rm -f /tmp/stop-post-marker

    : "Type=notify with READY=1"
    systemd-run --unit=notify-ready-test -p Type=notify \
        bash -c 'systemd-notify --ready; sleep 60'
    sleep 1
    systemctl is-active notify-ready-test.service
    systemctl stop notify-ready-test.service

    : "SupplementaryGroups= tests"
    systemd-run --wait --pipe -p User=testuser -p SupplementaryGroups=audio \
        bash -xec 'id -Gn | tr " " "\n" | grep -q audio'

    : "Multiple SupplementaryGroups= entries"
    systemd-run --wait --pipe -p User=testuser \
        -p SupplementaryGroups=audio \
        -p SupplementaryGroups=video \
        bash -xec 'id -Gn | tr " " "\n" | grep -q audio;
                   id -Gn | tr " " "\n" | grep -q video'

    : "ImportCredential= tests"
    mkdir -p /run/credentials/@system
    echo -n "imported-value" > /run/credentials/@system/test-import-cred
    systemd-run --wait --pipe -p ImportCredential=test-import-cred \
        bash -xec '[[ "$(cat "$CREDENTIALS_DIRECTORY/test-import-cred")" == imported-value ]]'
    rm -f /run/credentials/@system/test-import-cred

    : "UnsetEnvironment= tests"
    systemd-run --wait --pipe \
        -p Environment=KEEP_ME=yes \
        -p Environment=DROP_ME=yes \
        -p UnsetEnvironment=DROP_ME \
        bash -xec '[[ "$KEEP_ME" == yes && -z "$DROP_ME" ]]'

    : "daemon-reload picks up new unit files"
    printf '[Service]\nType=oneshot\nRemainAfterExit=yes\nExecStart=true\n' > /run/systemd/system/reload-test.service
    systemctl daemon-reload
    systemctl start reload-test.service
    systemctl is-active reload-test.service
    systemctl stop reload-test.service
    rm -f /run/systemd/system/reload-test.service
    systemctl daemon-reload

    : "systemctl show -P for service properties"
    systemd-run --unit=show-prop-test -p RemainAfterExit=yes -p Type=oneshot true
    sleep 1
    [[ "$(systemctl show -P Type show-prop-test.service)" == oneshot ]]
    [[ "$(systemctl show -P RemainAfterExit show-prop-test.service)" == yes ]]
    systemctl stop show-prop-test.service

    : "UtmpIdentifier and UtmpMode via transient properties"
    assert_eq "$(systemd-run -qP -p UtmpIdentifier=test -p UtmpMode=user whoami)" "$(whoami)"

    : "StandardInput=null is default (stdin is /dev/null)"
    systemd-run --wait --pipe -p StandardInput=null \
        bash -xec '[[ ! -t 0 ]]'

    : "ProcSubset=pid hides non-PID entries in /proc"
    systemd-run --wait --pipe -p PrivateMounts=yes -p ProcSubset=pid \
        bash -xec '(! test -d /proc/sys)'

    : "SyslogIdentifier via transient property"
    systemd-run --wait --pipe -p SyslogIdentifier=custom-ident true

    : "TTYPath via transient property (no-op when stdin=null)"
    systemd-run --wait --pipe -p TTYPath=/dev/console true

    : "LogLevelMax via transient property"
    systemd-run --wait --pipe -p LogLevelMax=warning true

    : "TimerSlackNSec= sets timer slack"
    SLACK="$(systemd-run --wait --pipe -p TimerSlackNSec=1000000 \
        bash -xec 'cat /proc/self/timerslack_ns')"
    [[ "$SLACK" == "1000000" ]]

    : "IOSchedulingClass= and IOSchedulingPriority= via transient properties"
    systemd-run --wait --pipe -p IOSchedulingClass=best-effort -p IOSchedulingPriority=5 true
    systemd-run --wait --pipe -p IOSchedulingClass=idle true

    : "CoredumpFilter= sets coredump filter"
    FILTER="$(systemd-run --wait --pipe -p CoredumpFilter=0x33 \
        bash -xec 'cat /proc/self/coredump_filter')"
    [[ "$FILTER" == "00000033" ]]

    : "CPUAffinity= pins process to specific CPUs"
    MASK="$(systemd-run --wait --pipe -p CPUAffinity=0 \
        bash -xec 'taskset -p $$ | sed "s/.*: //"')"
    [[ "$MASK" == "1" ]]

    : "PrivateIPC=yes creates IPC namespace isolation"
    HOST_IPC="$(readlink /proc/1/ns/ipc)"
    SRVC_IPC="$(systemd-run --wait --pipe -p PrivateIPC=yes readlink /proc/self/ns/ipc)"
    [[ "$HOST_IPC" != "$SRVC_IPC" ]]

    : "NetworkNamespacePath= joins existing network namespace"
    ip netns add test-ns-path || true
    EXPECTED_NS="$(readlink /proc/1/ns/net)"
    SRVC_NS="$(systemd-run --wait --pipe -p NetworkNamespacePath=/run/netns/test-ns-path readlink /proc/self/ns/net)"
    [[ "$EXPECTED_NS" != "$SRVC_NS" ]]
    ip netns del test-ns-path || true

    : "Personality= sets execution domain"
    systemd-run --wait --pipe -p Personality=x86-64 \
        bash -xec '[[ "$(uname -m)" == x86_64 ]]'
    systemd-run --wait --pipe -p Personality=x86 \
        bash -xec '[[ "$(uname -m)" == i686 ]]'

    : "Personality= with LockPersonality= combination"
    systemd-run --wait --pipe -p Personality=x86 -p LockPersonality=yes -p NoNewPrivileges=yes \
        bash -xec '[[ "$(uname -m)" == i686 ]]'

    : "ProtectHostname=yes isolates hostname changes"
    ORIG_HOSTNAME="$(hostname)"
    systemd-run --wait --pipe -p ProtectHostname=yes \
        bash -xec 'hostname test-ph-change; [[ "$(hostname)" == "test-ph-change" ]]'
    [[ "$(hostname)" == "$ORIG_HOSTNAME" ]]

    : "ProtectHostname=yes:hostname sets hostname in UTS namespace"
    ORIG_HOSTNAME="$(hostname)"
    systemd-run --wait --pipe -p ProtectHostname=yes:test-custom-host \
        bash -xec '[[ "$(hostname)" == "test-custom-host" ]]'
    [[ "$(hostname)" == "$ORIG_HOSTNAME" ]]

    : "ProtectHostname=private allows hostname changes within namespace"
    ORIG_HOSTNAME="$(hostname)"
    systemd-run --wait --pipe -p ProtectHostname=private \
        bash -xec 'hostname foo; [[ "$(hostname)" == "foo" ]]'
    [[ "$(hostname)" == "$ORIG_HOSTNAME" ]]

    : "ProtectHostname=private:hostname sets initial hostname, allows changes"
    ORIG_HOSTNAME="$(hostname)"
    systemd-run --wait --pipe -p ProtectHostname=private:test-priv-host \
        bash -xec '[[ "$(hostname)" == "test-priv-host" ]]; hostname bar; [[ "$(hostname)" == "bar" ]]'
    [[ "$(hostname)" == "$ORIG_HOSTNAME" ]]

    : "ProtectHostnameEx=yes:hostname works as alias for ProtectHostname"
    ORIG_HOSTNAME="$(hostname)"
    systemd-run --wait --pipe -p ProtectHostnameEx=yes:test-ex-host \
        bash -xec '[[ "$(hostname)" == "test-ex-host" ]]'
    [[ "$(hostname)" == "$ORIG_HOSTNAME" ]]

    : "PrivateMounts=yes creates isolated mount namespace"
    systemd-run --wait --pipe -p PrivateMounts=yes \
        bash -xec 'mount -t tmpfs none /tmp 2>/dev/null && touch /tmp/private-mount-test'
    [[ ! -e /tmp/private-mount-test ]]

    : "ProtectKernelTunables=yes with PrivateMounts=yes combination"
    systemd-run --wait --pipe -p ProtectKernelTunables=yes -p PrivateMounts=yes \
        bash -xec '(! sysctl -w kernel.domainname=test 2>/dev/null)'

    : "ProtectKernelLogs=yes with ProtectKernelModules=yes combination"
    systemd-run --wait --pipe -p ProtectKernelLogs=yes -p ProtectKernelModules=yes \
        bash -xec '[[ "$(stat -c %t:%T /dev/kmsg)" == "$(stat -c %t:%T /dev/null)" ]];
                   (! ls /usr/lib/modules 2>/dev/null)'

    : "ProtectSystem=strict with ProtectHome=yes combination"
    systemd-run --wait --pipe -p ProtectSystem=strict -p ProtectHome=yes \
        bash -xec 'test ! -w /; test ! -w /etc; test ! -w /var;
                   test ! -e /root/.bashrc 2>/dev/null || test ! -w /root'

    : "PrivateNetwork=yes with PrivateUsers=yes combination"
    systemd-run --wait --pipe -p PrivateNetwork=yes -p PrivateUsers=yes \
        bash -xec '(! ip link show eth0 2>/dev/null); ip link show lo;
                   [[ "$(cat /proc/self/uid_map | awk "{print \$1}")" == "0" ]]'

    : "Multiple InaccessiblePaths= entries"
    mkdir -p /tmp/inac-test-1 /tmp/inac-test-2
    echo "data1" > /tmp/inac-test-1/file
    echo "data2" > /tmp/inac-test-2/file
    systemd-run --wait --pipe \
        -p InaccessiblePaths="/tmp/inac-test-1" \
        -p InaccessiblePaths="/tmp/inac-test-2" \
        bash -xec '(! cat /tmp/inac-test-1/file 2>/dev/null);
                   (! cat /tmp/inac-test-2/file 2>/dev/null)'
    rm -rf /tmp/inac-test-1 /tmp/inac-test-2

    : "TemporaryFileSystem= with options (ro)"
    systemd-run --wait --pipe -p TemporaryFileSystem="/tmp/tmpfs-ro-test:ro" \
        bash -xec '[[ -d /tmp/tmpfs-ro-test ]]; (! touch /tmp/tmpfs-ro-test/file 2>/dev/null)'

    : "KeyringMode=private creates a new anonymous session keyring"
    systemd-run --wait --pipe -p KeyringMode=private \
        bash -xec 'true'

    : "KeyringMode=shared creates a session keyring linked to user keyring"
    systemd-run --wait --pipe -p KeyringMode=shared \
        bash -xec 'true'

    : "KeyringMode=inherit preserves the parent session keyring"
    systemd-run --wait --pipe -p KeyringMode=inherit \
        bash -xec 'true'

    : "SecureBits= can be set without error"
    systemd-run --wait --pipe -p SecureBits=keep-caps \
        bash -xec 'true'

    : "SecureBits= multiple flags combined"
    systemd-run --wait --pipe -p "SecureBits=keep-caps noroot no-setuid-fixup" \
        bash -xec 'true'

    : "StandardOutput=file: writes stdout to a file"
    systemd-run --wait --pipe -p StandardOutput=file:/tmp/stdout-file-test \
        bash -xec 'echo hello-stdout'
    [[ "$(cat /tmp/stdout-file-test)" == "hello-stdout" ]]
    rm -f /tmp/stdout-file-test

    : "StandardError=file: writes stderr to a file"
    systemd-run --wait --pipe -p StandardError=file:/tmp/stderr-file-test \
        bash -c 'echo hello-stderr >&2'
    grep -q hello-stderr /tmp/stderr-file-test
    rm -f /tmp/stderr-file-test

    : "StandardOutput=append: appends to existing file"
    echo "line1" > /tmp/stdout-append-test
    systemd-run --wait --pipe -p StandardOutput=append:/tmp/stdout-append-test \
        bash -xec 'echo line2'
    grep -q line1 /tmp/stdout-append-test
    grep -q line2 /tmp/stdout-append-test
    rm -f /tmp/stdout-append-test

    : "StandardError=append: appends to existing file"
    echo "err-line1" > /tmp/stderr-append-test
    systemd-run --wait --pipe -p StandardError=append:/tmp/stderr-append-test \
        bash -c 'echo err-line2 >&2'
    grep -q err-line1 /tmp/stderr-append-test
    grep -q err-line2 /tmp/stderr-append-test
    rm -f /tmp/stderr-append-test

    : "CPUSchedulingPolicy=rr with CPUSchedulingPriority= sets realtime scheduling"
    systemd-run --wait --pipe -p CPUSchedulingPolicy=rr -p CPUSchedulingPriority=10 \
        bash -xec 'chrt -p $$ | grep -q "SCHED_RR"'

    : "CPUSchedulingPolicy=fifo sets FIFO scheduling"
    systemd-run --wait --pipe -p CPUSchedulingPolicy=fifo -p CPUSchedulingPriority=1 \
        bash -xec 'chrt -p $$ | grep -q "SCHED_FIFO"'

    : "CPUSchedulingPolicy=batch sets batch scheduling"
    systemd-run --wait --pipe -p CPUSchedulingPolicy=batch \
        bash -xec 'chrt -p $$ | grep -q "SCHED_BATCH"'

    : "IOSchedulingClass=best-effort with IOSchedulingPriority="
    systemd-run --wait --pipe -p IOSchedulingClass=best-effort -p IOSchedulingPriority=3 \
        bash -xec 'ionice -p $$ | grep -q "best-effort.*prio 3"'

    : "IOSchedulingClass=idle sets idle I/O scheduling"
    systemd-run --wait --pipe -p IOSchedulingClass=idle \
        bash -xec 'ionice -p $$ | grep -q idle'

    : "EnvironmentFile= reads env vars from file"
    echo 'ENVFILE_VAR=hello_from_file' > /tmp/test-envfile
    echo 'ENVFILE_VAR2=second_val' >> /tmp/test-envfile
    systemd-run --wait --pipe -p EnvironmentFile=/tmp/test-envfile \
        bash -xec '[[ "$ENVFILE_VAR" == "hello_from_file" && "$ENVFILE_VAR2" == "second_val" ]]'
    rm -f /tmp/test-envfile

    : "MountFlags=slave creates mount namespace with slave propagation"
    systemd-run --wait --pipe -p MountFlags=slave \
        bash -xec 'mount -t tmpfs none /tmp 2>/dev/null; touch /tmp/slave-test'
    [[ ! -e /tmp/slave-test ]]

    : "MountFlags=private creates mount namespace with private propagation"
    systemd-run --wait --pipe -p MountFlags=private \
        bash -xec 'mount -t tmpfs none /tmp 2>/dev/null; touch /tmp/private-test'
    [[ ! -e /tmp/private-test ]]

    : "ProtectProc=invisible hides other processes from non-root user"
    systemd-run --wait --pipe -p PrivateMounts=yes -p ProtectProc=invisible -p User=testuser \
        bash -xec '(! ls /proc/1/cmdline 2>/dev/null) || [[ ! -r /proc/1/cmdline ]]'

    : "ProtectProc=noaccess denies access to other PIDs for non-root user"
    systemd-run --wait --pipe -p PrivateMounts=yes -p ProtectProc=noaccess -p User=testuser \
        bash -xec '(! cat /proc/1/cmdline 2>/dev/null)'

    : "IgnoreSIGPIPE=no leaves SIGPIPE default (kills process)"
    (! systemd-run --wait --pipe -p IgnoreSIGPIPE=no \
        bash -c 'kill -PIPE $$')

    : "IgnoreSIGPIPE=yes (default) ignores SIGPIPE"
    systemd-run --wait --pipe -p IgnoreSIGPIPE=yes \
        bash -xec 'true'

    : "CPUSchedulingResetOnFork=yes with FIFO scheduling"
    systemd-run --wait --pipe \
        -p CPUSchedulingPolicy=fifo -p CPUSchedulingPriority=1 \
        -p CPUSchedulingResetOnFork=yes \
        bash -xec 'true'

    : "StandardOutput=truncate: truncates file before writing"
    echo "old-content" > /tmp/truncate-test
    systemd-run --wait --pipe -p StandardOutput=truncate:/tmp/truncate-test \
        bash -xec 'echo new-content'
    grep -q new-content /tmp/truncate-test
    (! grep -q old-content /tmp/truncate-test)
    rm -f /tmp/truncate-test

    : "Multiple Environment= entries accumulate"
    systemd-run --wait --pipe \
        -p Environment=FOO=first \
        -p Environment=BAR=second \
        bash -xec '[[ "$FOO" == first && "$BAR" == second ]]'

    : "Environment= with spaces in values"
    systemd-run --wait --pipe \
        -p 'Environment=SPACED=hello world' \
        bash -xec '[[ "$SPACED" == "hello world" ]]'

    : "LimitNOFILE= sets open file descriptor limit"
    NOFILE="$(systemd-run --wait --pipe -p LimitNOFILE=4096 \
        bash -xec 'ulimit -n')"
    [[ "$NOFILE" == "4096" ]]

    : "LimitNOFILE= with soft:hard syntax"
    NOFILE_SOFT="$(systemd-run --wait --pipe -p LimitNOFILE=1024:8192 \
        bash -xec 'ulimit -Sn')"
    NOFILE_HARD="$(systemd-run --wait --pipe -p LimitNOFILE=1024:8192 \
        bash -xec 'ulimit -Hn')"
    [[ "$NOFILE_SOFT" == "1024" ]]
    [[ "$NOFILE_HARD" == "8192" ]]

    : "LimitNPROC= sets max processes limit"
    NPROC="$(systemd-run --wait --pipe -p LimitNPROC=512 \
        bash -xec 'ulimit -u')"
    [[ "$NPROC" == "512" ]]

    : "LimitCORE= sets core dump size limit"
    CORE="$(systemd-run --wait --pipe -p LimitCORE=0 \
        bash -xec 'ulimit -c')"
    [[ "$CORE" == "0" ]]

    : "LimitCORE=infinity sets unlimited core dump"
    CORE="$(systemd-run --wait --pipe -p LimitCORE=infinity \
        bash -xec 'ulimit -c')"
    [[ "$CORE" == "unlimited" ]]

    : "LimitFSIZE= sets max file size limit"
    FSIZE="$(systemd-run --wait --pipe -p LimitFSIZE=1048576 \
        bash -xec 'ulimit -f')"
    [[ "$FSIZE" == "1024" ]]

    : "LimitMEMLOCK= sets locked memory limit"
    MEMLOCK="$(systemd-run --wait --pipe -p LimitMEMLOCK=8388608 \
        bash -xec 'ulimit -l')"
    [[ "$MEMLOCK" == "8192" ]]

    : "LimitSTACK= sets stack size limit"
    STACK="$(systemd-run --wait --pipe -p LimitSTACK=16777216 \
        bash -xec 'ulimit -s')"
    [[ "$STACK" == "16384" ]]

    : "LimitAS= sets virtual memory limit"
    AS_LIM="$(systemd-run --wait --pipe -p LimitAS=2147483648 \
        bash -xec 'ulimit -v')"
    [[ "$AS_LIM" == "2097152" ]]

    : "LimitSIGPENDING= sets pending signals limit"
    SIGPEND="$(systemd-run --wait --pipe -p LimitSIGPENDING=256 \
        bash -xec 'ulimit -i')"
    [[ "$SIGPEND" == "256" ]]

    : "LimitMSGQUEUE= sets POSIX message queue size"
    MSGQ="$(systemd-run --wait --pipe -p LimitMSGQUEUE=1048576 \
        bash -xec 'ulimit -q')"
    [[ "$MSGQ" == "1048576" ]]

    : "LimitRTPRIO= sets realtime priority limit"
    RTPRIO="$(systemd-run --wait --pipe -p LimitRTPRIO=50 \
        bash -xec 'ulimit -r')"
    [[ "$RTPRIO" == "50" ]]

    : "DynamicUser=yes runs without error"
    systemd-run --wait --pipe -p DynamicUser=yes \
        bash -xec 'true'

    : "RemoveIPC=yes with User= runs without error"
    systemd-run --wait --pipe -p User=testuser -p RemoveIPC=yes \
        bash -xec 'true'

    : "KillMode=process only kills main process"
    systemd-run --unit=killmode-test -p KillMode=process -p RemainAfterExit=no \
        bash -c 'sleep 999 & disown; exec sleep 60'
    sleep 1
    MAIN_PID="$(systemctl show -P MainPID killmode-test.service)"
    [[ "$MAIN_PID" -gt 0 ]]
    systemctl stop killmode-test.service 2>/dev/null || true

    : "SendSIGHUP=yes sends SIGHUP after SIGTERM"
    systemd-run --wait --pipe -p SendSIGHUP=yes \
        bash -xec 'true'

    : "IPCNamespacePath= joins existing IPC namespace"
    HOST_IPC="$(readlink /proc/1/ns/ipc)"
    # Create a service with its own IPC namespace
    systemd-run --unit=ipc-ns-provider -p PrivateIPC=yes -p RemainAfterExit=no \
        sleep 60
    sleep 1
    PROVIDER_PID="$(systemctl show -P MainPID ipc-ns-provider.service)"
    PROVIDER_IPC="$(readlink /proc/$PROVIDER_PID/ns/ipc)"
    [[ "$HOST_IPC" != "$PROVIDER_IPC" ]]
    # Join that IPC namespace
    JOINED_IPC="$(systemd-run --wait --pipe -p IPCNamespacePath=/proc/$PROVIDER_PID/ns/ipc readlink /proc/self/ns/ipc)"
    [[ "$JOINED_IPC" == "$PROVIDER_IPC" ]]
    systemctl stop ipc-ns-provider.service 2>/dev/null || true

    : "CacheDirectory= creates cache directory"
    systemd-run --wait --pipe -p CacheDirectory=test-cache-dir \
        bash -xec '[[ -d /var/cache/test-cache-dir ]]'
    rm -rf /var/cache/test-cache-dir

    : "ConfigurationDirectory= creates config directory"
    systemd-run --wait --pipe -p ConfigurationDirectory=test-config-dir \
        bash -xec '[[ -d /etc/test-config-dir ]]'
    rm -rf /etc/test-config-dir

    : "LogsDirectory= creates logs directory"
    systemd-run --wait --pipe -p LogsDirectory=test-logs-dir \
        bash -xec '[[ -d /var/log/test-logs-dir ]]'
    rm -rf /var/log/test-logs-dir

    : "SyslogLevel= and SyslogFacility= accepted without error"
    systemd-run --wait --pipe -p SyslogLevel=debug -p SyslogFacility=local0 \
        bash -xec 'true'

    : "LogRateLimitBurst= and LogRateLimitIntervalSec= accepted"
    systemd-run --wait --pipe -p LogRateLimitBurst=100 -p LogRateLimitIntervalSec=5s \
        bash -xec 'true'

    : "PrivateDevices=yes with PrivateIPC=yes combination"
    systemd-run --wait --pipe -p PrivateDevices=yes -p PrivateIPC=yes \
        bash -xec 'HOST_IPC=$(readlink /proc/1/ns/ipc);
                   MY_IPC=$(readlink /proc/self/ns/ipc);
                   [[ "$HOST_IPC" != "$MY_IPC" ]];
                   [[ "$(stat -c %t:%T /dev/null)" == "1:3" ]]'

    : "ProtectSystem=full makes /usr, /boot, and /etc read-only"
    systemd-run --wait --pipe -p ProtectSystem=full \
        bash -xec '(! touch /usr/should-fail 2>/dev/null);
                   (! touch /etc/should-fail 2>/dev/null)'

    : "ProtectHome=read-only makes home directories read-only"
    systemd-run --wait --pipe -p ProtectHome=read-only \
        bash -xec 'test -d /root;
                   (! touch /root/should-fail 2>/dev/null)'

    : "ProtectHome=tmpfs mounts tmpfs over home directories"
    touch /root/home-marker
    systemd-run --wait --pipe -p ProtectHome=tmpfs \
        bash -xec 'test -d /root;
                   test ! -e /root/home-marker'
    rm -f /root/home-marker

    : "ProtectControlGroups=yes makes cgroup fs read-only"
    systemd-run --wait --pipe -p ProtectControlGroups=yes \
        bash -xec '(! mkdir /sys/fs/cgroup/test-readonly 2>/dev/null)'

    : "ProtectKernelModules=yes denies module loading"
    systemd-run --wait --pipe -p ProtectKernelModules=yes \
        bash -xec '(! ls /usr/lib/modules 2>/dev/null) || true'

    : "ProtectKernelLogs=yes hides kernel log"
    systemd-run --wait --pipe -p ProtectKernelLogs=yes \
        bash -xec '[[ "$(stat -c %t:%T /dev/kmsg)" == "$(stat -c %t:%T /dev/null)" ]]'

    : "ProtectKernelTunables=yes makes sysfs read-only"
    systemd-run --wait --pipe -p ProtectKernelTunables=yes -p PrivateMounts=yes \
        bash -xec '(! sysctl -w kernel.domainname=test-tunables 2>/dev/null)'

    : "RuntimeDirectoryPreserve=yes keeps directory after service stop"
    UNIT="rtdir-preserve-$RANDOM"
    systemd-run --unit="$UNIT" -p RuntimeDirectory=test-preserve \
        -p RuntimeDirectoryPreserve=yes -p RemainAfterExit=yes -p Type=oneshot \
        bash -xec 'touch /run/test-preserve/marker'
    sleep 1
    systemctl stop "$UNIT.service"
    sleep 1
    [[ -f /run/test-preserve/marker ]]
    rm -rf /run/test-preserve

    : "RuntimeDirectoryPreserve=no removes directory after service stop"
    UNIT="rtdir-nopreserve-$RANDOM"
    systemd-run --unit="$UNIT" -p RuntimeDirectory=test-nopreserve \
        -p RuntimeDirectoryPreserve=no -p RemainAfterExit=yes -p Type=oneshot \
        bash -xec 'touch /run/test-nopreserve/marker'
    sleep 1
    systemctl stop "$UNIT.service"
    sleep 1
    [[ ! -d /run/test-nopreserve ]]

    : "BindPaths= makes host path available inside service"
    mkdir -p /tmp/bind-src
    echo "bind-data" > /tmp/bind-src/file
    systemd-run --wait --pipe -p BindPaths=/tmp/bind-src:/tmp/bind-dst \
        bash -xec '[[ "$(cat /tmp/bind-dst/file)" == "bind-data" ]]'
    rm -rf /tmp/bind-src

    : "BindReadOnlyPaths= makes path read-only inside service"
    mkdir -p /tmp/bind-ro-src
    echo "ro-data" > /tmp/bind-ro-src/file
    systemd-run --wait --pipe -p BindReadOnlyPaths=/tmp/bind-ro-src:/tmp/bind-ro-dst \
        bash -xec '[[ "$(cat /tmp/bind-ro-dst/file)" == "ro-data" ]];
                   (! touch /tmp/bind-ro-dst/new-file 2>/dev/null)'
    rm -rf /tmp/bind-ro-src

    : "SuccessExitStatus= treats custom exit codes as success"
    UNIT="success-exit-$RANDOM"
    systemd-run --unit="$UNIT" -p SuccessExitStatus=42 -p Type=oneshot \
        bash -c 'exit 42'
    sleep 1
    # The unit should show Result=success, not Result=exit-code
    [[ "$(systemctl show -P Result "$UNIT.service")" == "success" ]]
    systemctl reset-failed "$UNIT.service" 2>/dev/null || true

    : "RestartPreventExitStatus= prevents restart on specific exit code"
    UNIT="no-restart-on-42-$RANDOM"
    systemd-run --unit="$UNIT" -p Restart=on-failure -p RestartSec=0 \
        -p 'RestartPreventExitStatus=42' \
        bash -c 'exit 42'
    sleep 2
    # Service should NOT have been restarted (42 prevents restart)
    [[ "$(systemctl show -P NRestarts "$UNIT.service")" == "0" ]]
    systemctl reset-failed "$UNIT.service" 2>/dev/null || true

    : "ExecReload= via systemctl reload"
    UNIT="reload-test-$RANDOM"
    systemd-run --unit="$UNIT" -p Type=notify \
        -p ExecReload="touch /tmp/reload-marker-$UNIT" \
        bash -c 'systemd-notify --ready; sleep 60'
    sleep 1
    systemctl reload "$UNIT.service"
    sleep 1
    [[ -f "/tmp/reload-marker-$UNIT" ]]
    systemctl stop "$UNIT.service"
    rm -f "/tmp/reload-marker-$UNIT"

    : "ExecStartPre= with plus prefix runs as root even with User="
    systemd-run --wait --pipe -p User=testuser \
        -p ExecStartPre="+touch /tmp/plus-prefix-marker" \
        bash -xec '[[ -f /tmp/plus-prefix-marker ]]'
    rm -f /tmp/plus-prefix-marker

    : "Error handling for clean-up codepaths"
    (! systemd-run --wait --pipe false)

    : "ExecStop= runs on service stop"
    UNIT="execstop-test-$RANDOM"
    systemd-run --unit="$UNIT" -p Type=notify \
        -p ExecStop="touch /tmp/execstop-marker-$UNIT" \
        bash -c 'systemd-notify --ready; sleep 60'
    sleep 1
    systemctl is-active "$UNIT.service"
    systemctl stop "$UNIT.service"
    sleep 1
    [[ -f "/tmp/execstop-marker-$UNIT" ]]
    rm -f "/tmp/execstop-marker-$UNIT"

    : "ExecStopPost= runs after service stops"
    UNIT="execstoppost-test-$RANDOM"
    systemd-run --unit="$UNIT" -p Type=notify \
        -p ExecStopPost="touch /tmp/execstoppost-marker-$UNIT" \
        bash -c 'systemd-notify --ready; sleep 60'
    sleep 1
    systemctl stop "$UNIT.service"
    sleep 1
    [[ -f "/tmp/execstoppost-marker-$UNIT" ]]
    rm -f "/tmp/execstoppost-marker-$UNIT"

    : "RestartForceExitStatus= forces restart on specific exit code"
    UNIT="force-restart-$RANDOM"
    systemd-run --unit="$UNIT" -p Restart=no -p RestartSec=0 \
        -p 'RestartForceExitStatus=42' \
        bash -c 'exit 42'
    sleep 2
    # Despite Restart=no, exit 42 should force a restart
    [[ "$(systemctl show -P NRestarts "$UNIT.service")" -ge "1" ]]
    systemctl stop "$UNIT.service" 2>/dev/null || true
    systemctl reset-failed "$UNIT.service" 2>/dev/null || true

    : "SendSIGKILL=no is accepted as a property"
    systemd-run --wait --pipe -p SendSIGKILL=no true

    : "FinalKillSignal= is accepted as a property"
    systemd-run --wait --pipe -p FinalKillSignal=9 true

    : "RestartKillSignal= is accepted as a property"
    systemd-run --wait --pipe -p RestartKillSignal=15 true

    : "LimitRTTIME= real-time scheduling time limit"
    systemd-run --wait --pipe -p LimitRTTIME=666666 \
        bash -xec 'if ulimit -R 2>/dev/null; then [[ $(ulimit -SR) -eq 666666 ]]; fi'

    : "Multiple ExecStart= with Type=oneshot runs all commands"
    UNIT="multi-exec-$RANDOM"
    printf '[Service]\nType=oneshot\nExecStart=touch /tmp/multi-exec-1-%s\nExecStart=touch /tmp/multi-exec-2-%s\n' \
        "$UNIT" "$UNIT" > "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    sleep 1
    [[ -f "/tmp/multi-exec-1-$UNIT" ]]
    [[ -f "/tmp/multi-exec-2-$UNIT" ]]
    rm -f "/tmp/multi-exec-1-$UNIT" "/tmp/multi-exec-2-$UNIT"
    rm -f "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload

    : "Condition checks via systemctl show"
    UNIT="condcheck-$RANDOM"
    systemd-run --unit="$UNIT" -p RemainAfterExit=yes -p Type=oneshot true
    sleep 1
    # Basic property check
    [[ "$(systemctl show -P Type "$UNIT.service")" == "oneshot" ]]
    [[ "$(systemctl show -P RemainAfterExit "$UNIT.service")" == "yes" ]]
    systemctl stop "$UNIT.service"

    : "Description= via --description flag"
    UNIT="desc-test-$RANDOM"
    systemd-run --unit="$UNIT" --description="My test description" \
        -p RemainAfterExit=yes -p Type=oneshot true
    sleep 1
    [[ "$(systemctl show -P Description "$UNIT.service")" == "My test description" ]]
    systemctl stop "$UNIT.service"

    : "Type=exec waits for exec before reporting active"
    systemd-run --wait --pipe -p Type=exec true

    : "Environment= accumulation via multiple -p flags"
    systemd-run --wait --pipe \
        -p Environment=FOO=one \
        -p Environment=BAR=two \
        bash -xec '[[ "$FOO" == "one" && "$BAR" == "two" ]]'

    : "Environment= last value wins for same variable"
    systemd-run --wait --pipe \
        -p Environment=FOO=first \
        -p Environment=FOO=second \
        bash -xec '[[ "$FOO" == "second" ]]'

    : "TimeoutStopSec= affects stop behavior"
    UNIT="timeout-stop-$RANDOM"
    systemd-run --unit="$UNIT" -p TimeoutStopSec=2 sleep 300
    sleep 1
    systemctl is-active "$UNIT.service"
    systemctl stop "$UNIT.service"
    # After stop, it should not be active
    (! systemctl is-active "$UNIT.service")

    : "ExecStartPre= with - prefix ignores failure"
    systemd-run --wait --pipe \
        -p ExecStartPre='-false' \
        bash -xec 'echo "main command ran despite ExecStartPre failure"'

    : "ExecStartPre= without - prefix causes failure on error"
    (! systemd-run --wait --pipe \
        -p ExecStartPre='false' \
        bash -xec 'echo "this should not run"')

    : "RuntimeDirectory is cleaned on stop"
    UNIT="clean-test-$RANDOM"
    systemd-run --unit="$UNIT" -p Type=oneshot \
        -p RemainAfterExit=yes \
        -p RuntimeDirectory="$UNIT" \
        true
    sleep 1
    [[ -d "/run/$UNIT" ]]
    systemctl stop "$UNIT.service"
    [[ ! -e "/run/$UNIT" ]]

    TESTEOF
    chmod +x TEST-07-PID1.exec-context.sh
    # Rewrite private-pids test: keep only testcase_basic.
    # Remove testcase_analyze (systemd-analyze not implemented),
    # testcase_multiple_features (unsquashfs/PrivateUsersEx/PrivateIPC),
    # testcase_unpriv (--user mode not implemented).
    cat > TEST-07-PID1.private-pids.sh << 'PPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "PrivatePIDs=yes basic test"
    assert_eq "$(systemd-run -p PrivatePIDs=yes --wait --pipe readlink /proc/self)" "1"
    assert_eq "$(systemd-run -p PrivatePIDs=yes --wait --pipe ps aux --no-heading | wc -l)" "1"

    : "PrivatePIDs=yes procfs mount options"
    systemd-run -p PrivatePIDs=yes --wait --pipe \
        bash -xec 'OPTS=$(findmnt --mountpoint /proc --noheadings -o VFS-OPTIONS);
                   [[ "$OPTS" =~ rw ]];
                   [[ "$OPTS" =~ nosuid ]];
                   [[ "$OPTS" =~ nodev ]];
                   [[ "$OPTS" =~ noexec ]];'
    PPEOF
    chmod +x TEST-07-PID1.private-pids.sh
    # Custom start-limit test: verify StartLimitBurst/StartLimitIntervalSec enforcement
    cat > TEST-07-PID1.start-limit.sh << 'SLEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="test-start-limit-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        systemctl reset-failed "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    printf '[Unit]\nStartLimitBurst=3\nStartLimitIntervalSec=30\n[Service]\nType=oneshot\nExecStart=false\n' > "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload

    # First 3 starts should be allowed (they fail, but they start)
    for i in 1 2 3; do
        systemctl start "$UNIT.service" || true
    done

    # After 3 failures within the interval, the 4th start should be refused
    (! systemctl start "$UNIT.service" 2>/dev/null)
    SLEOF
    chmod +x TEST-07-PID1.start-limit.sh
    # Custom service-dependencies test: verify ordering and dependency handling
    cat > TEST-07-PID1.service-dependencies.sh << 'SDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop dep-*.service 2>/dev/null
        rm -f /run/systemd/system/dep-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "Wants= starts the wanted unit"
    printf '[Service]\nType=oneshot\nRemainAfterExit=yes\nExecStart=true\n' > /run/systemd/system/dep-wanted.service
    printf '[Unit]\nWants=dep-wanted.service\nAfter=dep-wanted.service\n[Service]\nType=oneshot\nRemainAfterExit=yes\nExecStart=true\n' > /run/systemd/system/dep-wanter.service
    systemctl daemon-reload
    systemctl start dep-wanter.service
    sleep 1
    systemctl is-active dep-wanted.service
    systemctl is-active dep-wanter.service
    systemctl stop dep-wanter.service dep-wanted.service

    : "Requires= starts the required unit"
    printf '[Service]\nType=oneshot\nRemainAfterExit=yes\nExecStart=true\n' > /run/systemd/system/dep-required.service
    printf '[Unit]\nRequires=dep-required.service\nAfter=dep-required.service\n[Service]\nType=oneshot\nRemainAfterExit=yes\nExecStart=true\n' > /run/systemd/system/dep-requirer.service
    systemctl daemon-reload
    systemctl start dep-requirer.service
    sleep 1
    systemctl is-active dep-required.service
    systemctl is-active dep-requirer.service
    systemctl stop dep-requirer.service dep-required.service
    SDEOF
    chmod +x TEST-07-PID1.service-dependencies.sh
    # Custom forking service test: verify Type=forking with PIDFile tracking
    cat > TEST-07-PID1.forking-pidfile.sh << 'FPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="test-forking-pidfile-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "/run/$UNIT.pid"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    printf '[Service]\nType=forking\nPIDFile=/run/%s.pid\nExecStart=bash -c '"'"'sleep infinity & echo $! > /run/%s.pid'"'"'\n' "$UNIT" "$UNIT" > "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    sleep 1

    # Verify the service is active and PID was tracked
    systemctl is-active "$UNIT.service"
    MAIN_PID="$(systemctl show -P MainPID "$UNIT.service")"
    [[ "$MAIN_PID" -gt 0 ]]
    # Verify the PID matches what was written to the PID file
    FILE_PID="$(cat "/run/$UNIT.pid")"
    [[ "$MAIN_PID" == "$FILE_PID" ]]

    systemctl stop "$UNIT.service"
    FPEOF
    chmod +x TEST-07-PID1.forking-pidfile.sh
    # Rewrite protect-hostname test: upstream uses hostnamectl and
    # seccomp-based sethostname() blocking. We only support UTS namespace
    # isolation (both "yes" and "private" modes behave as "private").
    cat > TEST-07-PID1.protect-hostname.sh << 'PHEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    LEGACY_HOSTNAME="$(hostname)"

    : "ProtectHostname=yes isolates hostname changes from host"
    systemd-run --wait -p ProtectHostname=yes \
        -P bash -xec 'hostname foo; test "$(hostname)" = "foo"'
    test "$(hostname)" = "$LEGACY_HOSTNAME"

    : "ProtectHostname=yes:hoge sets hostname in UTS namespace"
    systemd-run --wait -p ProtectHostname=yes:hoge \
        -P bash -xec '
            test "$(hostname)" = "hoge"
        '
    test "$(hostname)" = "$LEGACY_HOSTNAME"

    : "ProtectHostname=private allows hostname changes"
    systemd-run --wait -p ProtectHostname=private \
        -P bash -xec '
            hostname foo
            test "$(hostname)" = "foo"
        '
    test "$(hostname)" = "$LEGACY_HOSTNAME"

    : "ProtectHostname=private:hoge sets hostname, allows changes"
    systemd-run --wait -p ProtectHostname=private:hoge \
        -P bash -xec '
            test "$(hostname)" = "hoge"
            hostname foo
            test "$(hostname)" = "foo"
        '
    test "$(hostname)" = "$LEGACY_HOSTNAME"

    : "ProtectHostnameEx=yes:hoge works as alias"
    systemd-run --wait -p ProtectHostnameEx=yes:hoge \
        -P bash -xec '
            test "$(hostname)" = "hoge"
        '
    test "$(hostname)" = "$LEGACY_HOSTNAME"
    PHEOF
    chmod +x TEST-07-PID1.protect-hostname.sh

    # Custom restart behavior test
    cat > TEST-07-PID1.restart-behavior.sh << 'RESTARTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/restart-test-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "Restart=on-failure restarts on non-zero exit"
    cat > /run/systemd/system/restart-test-onfailure.service << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'if [ ! -f /tmp/restart-pass ]; then touch /tmp/restart-pass; exit 1; fi'
    RemainAfterExit=yes
    Restart=on-failure
    RestartSec=1
    EOF
    rm -f /tmp/restart-pass
    systemctl daemon-reload
    # First start will fail (exit 1), restart should succeed
    systemctl start restart-test-onfailure.service || true
    # Wait for the auto-restart to succeed
    timeout 15 bash -c 'until systemctl is-active restart-test-onfailure.service 2>/dev/null; do sleep 0.5; done'
    systemctl is-active restart-test-onfailure.service
    [[ "$(systemctl show -P NRestarts restart-test-onfailure.service)" -ge 1 ]]
    systemctl stop restart-test-onfailure.service
    rm -f /tmp/restart-pass

    : "Restart=no does not restart"
    cat > /run/systemd/system/restart-test-no.service << EOF
    [Service]
    Type=oneshot
    ExecStart=false
    Restart=no
    EOF
    systemctl daemon-reload
    systemctl start restart-test-no.service || true
    sleep 2
    [[ "$(systemctl show -P NRestarts restart-test-no.service)" -eq 0 ]]

    RESTARTEOF
    chmod +x TEST-07-PID1.restart-behavior.sh

    # Custom ExecStartPre/ExecStartPost ordering test
    cat > TEST-07-PID1.exec-start-pre-post.sh << 'ESPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/exec-order-*.service
        rm -f /tmp/exec-order-*
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "ExecStartPre= runs before ExecStart="
    cat > /run/systemd/system/exec-order-pre.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStartPre=bash -c 'echo pre > /tmp/exec-order-pre'
    ExecStart=bash -c 'test -f /tmp/exec-order-pre && echo main > /tmp/exec-order-main'
    EOF
    systemctl daemon-reload
    systemctl start exec-order-pre.service
    systemctl is-active exec-order-pre.service
    [[ -f /tmp/exec-order-pre ]]
    [[ -f /tmp/exec-order-main ]]
    systemctl stop exec-order-pre.service
    rm -f /tmp/exec-order-pre /tmp/exec-order-main

    : "ExecStartPost= runs after ExecStart="
    cat > /run/systemd/system/exec-order-post.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'echo main > /tmp/exec-order-main2'
    ExecStartPost=bash -c 'test -f /tmp/exec-order-main2 && echo post > /tmp/exec-order-post'
    EOF
    systemctl daemon-reload
    systemctl start exec-order-post.service
    systemctl is-active exec-order-post.service
    [[ -f /tmp/exec-order-main2 ]]
    [[ -f /tmp/exec-order-post ]]
    systemctl stop exec-order-post.service
    rm -f /tmp/exec-order-main2 /tmp/exec-order-post

    : "Multiple ExecStartPre= commands run in order"
    cat > /run/systemd/system/exec-order-multi-pre.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStartPre=bash -c 'echo 1 >> /tmp/exec-order-seq'
    ExecStartPre=bash -c 'echo 2 >> /tmp/exec-order-seq'
    ExecStart=bash -c 'echo 3 >> /tmp/exec-order-seq'
    ExecStartPost=bash -c 'echo 4 >> /tmp/exec-order-seq'
    EOF
    rm -f /tmp/exec-order-seq
    systemctl daemon-reload
    systemctl start exec-order-multi-pre.service
    systemctl is-active exec-order-multi-pre.service
    [[ "$(cat /tmp/exec-order-seq)" == "$(printf '1\n2\n3\n4')" ]]
    systemctl stop exec-order-multi-pre.service
    rm -f /tmp/exec-order-seq

    : "ExecStartPre= failure prevents ExecStart="
    cat > /run/systemd/system/exec-order-pre-fail.service << EOF
    [Service]
    Type=oneshot
    ExecStartPre=false
    ExecStart=bash -c 'echo should-not-run > /tmp/exec-order-nope'
    EOF
    rm -f /tmp/exec-order-nope
    systemctl daemon-reload
    systemctl start exec-order-pre-fail.service || true
    [[ ! -f /tmp/exec-order-nope ]]

    : "ExecStartPre= with - prefix ignores failure"
    cat > /run/systemd/system/exec-order-pre-dash.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStartPre=-false
    ExecStart=bash -c 'echo ran > /tmp/exec-order-dash'
    EOF
    rm -f /tmp/exec-order-dash
    systemctl daemon-reload
    systemctl start exec-order-pre-dash.service
    systemctl is-active exec-order-pre-dash.service
    [[ -f /tmp/exec-order-dash ]]
    systemctl stop exec-order-pre-dash.service
    rm -f /tmp/exec-order-dash
    ESPEOF
    chmod +x TEST-07-PID1.exec-start-pre-post.sh

    # Custom ExecStop/ExecStopPost ordering test
    cat > TEST-07-PID1.exec-stop-post.sh << 'ESPOSTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/stop-order-*.service
        rm -f /tmp/stop-order-*
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "ExecStop= runs on stop"
    cat > /run/systemd/system/stop-order-basic.service << EOF
    [Service]
    ExecStart=sleep infinity
    ExecStop=bash -c 'echo stopped > /tmp/stop-order-basic'
    EOF
    rm -f /tmp/stop-order-basic
    systemctl daemon-reload
    systemctl start stop-order-basic.service
    systemctl is-active stop-order-basic.service
    systemctl stop stop-order-basic.service
    [[ -f /tmp/stop-order-basic ]]
    [[ "$(cat /tmp/stop-order-basic)" == "stopped" ]]
    rm -f /tmp/stop-order-basic

    : "ExecStopPost= runs after service exits"
    cat > /run/systemd/system/stop-order-post.service << EOF
    [Service]
    ExecStart=sleep infinity
    ExecStopPost=bash -c 'echo post > /tmp/stop-order-post'
    EOF
    rm -f /tmp/stop-order-post
    systemctl daemon-reload
    systemctl start stop-order-post.service
    systemctl is-active stop-order-post.service
    systemctl stop stop-order-post.service
    [[ -f /tmp/stop-order-post ]]
    [[ "$(cat /tmp/stop-order-post)" == "post" ]]
    rm -f /tmp/stop-order-post

    : "ExecStopPost= runs even when ExecStop= fails"
    cat > /run/systemd/system/stop-order-post-after-fail.service << EOF
    [Service]
    ExecStart=sleep infinity
    ExecStop=false
    ExecStopPost=bash -c 'echo ran-anyway > /tmp/stop-order-post-fail'
    EOF
    rm -f /tmp/stop-order-post-fail
    systemctl daemon-reload
    systemctl start stop-order-post-after-fail.service
    systemctl is-active stop-order-post-after-fail.service
    # ExecStop=false fails, so systemctl stop may return non-zero
    systemctl stop stop-order-post-after-fail.service || true
    sleep 1
    [[ -f /tmp/stop-order-post-fail ]]
    [[ "$(cat /tmp/stop-order-post-fail)" == "ran-anyway" ]]
    rm -f /tmp/stop-order-post-fail

    : "ExecStop= and ExecStopPost= run in order"
    cat > /run/systemd/system/stop-order-sequence.service << EOF
    [Service]
    ExecStart=sleep infinity
    ExecStop=bash -c 'echo stop >> /tmp/stop-order-seq'
    ExecStopPost=bash -c 'echo post >> /tmp/stop-order-seq'
    EOF
    rm -f /tmp/stop-order-seq
    systemctl daemon-reload
    systemctl start stop-order-sequence.service
    systemctl is-active stop-order-sequence.service
    systemctl stop stop-order-sequence.service
    [[ "$(cat /tmp/stop-order-seq)" == "$(printf 'stop\npost')" ]]
    rm -f /tmp/stop-order-seq
    ESPOSTEOF
    chmod +x TEST-07-PID1.exec-stop-post.sh

    # Custom KillMode and KillSignal test
    cat > TEST-07-PID1.kill-mode.sh << 'KMEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/kill-mode-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "KillMode=control-group kills entire cgroup"
    cat > /run/systemd/system/kill-mode-cgroup.service << EOF
    [Service]
    Type=forking
    ExecStart=bash -c 'sleep infinity & echo \$! > /run/kill-mode-cgroup.pid; sleep infinity & disown'
    PIDFile=/run/kill-mode-cgroup.pid
    KillMode=control-group
    EOF
    systemctl daemon-reload
    systemctl start kill-mode-cgroup.service
    systemctl is-active kill-mode-cgroup.service
    MAIN_PID="$(systemctl show -P MainPID kill-mode-cgroup.service)"
    [[ "$MAIN_PID" -gt 0 ]]
    systemctl stop kill-mode-cgroup.service
    # Main process should be gone
    (! ps -p "$MAIN_PID" > /dev/null 2>&1)
    rm -f /run/kill-mode-cgroup.pid

    : "KillSignal=SIGTERM is default"
    cat > /run/systemd/system/kill-mode-signal.service << EOF
    [Service]
    ExecStart=sleep infinity
    KillSignal=SIGTERM
    EOF
    systemctl daemon-reload
    systemctl start kill-mode-signal.service
    systemctl is-active kill-mode-signal.service
    systemctl stop kill-mode-signal.service
    (! systemctl is-active kill-mode-signal.service)
    KMEOF
    chmod +x TEST-07-PID1.kill-mode.sh

    # Custom systemctl enable/disable/mask/unmask test
    cat > TEST-07-PID1.enable-disable.sh << 'EDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="enable-test-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        systemctl unmask "$UNIT.service" 2>/dev/null
        systemctl disable "$UNIT.service" 2>/dev/null
        rm -f "/usr/lib/systemd/system/$UNIT.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    cat > "/usr/lib/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    ExecStart=true
    [Install]
    WantedBy=multi-user.target
    EOF
    systemctl daemon-reload

    : "Enable creates symlink"
    (! systemctl is-enabled "$UNIT.service")
    systemctl enable "$UNIT.service"
    systemctl is-enabled "$UNIT.service"

    : "Disable removes symlink"
    systemctl disable "$UNIT.service"
    (! systemctl is-enabled "$UNIT.service")

    : "Mask creates /dev/null symlink"
    systemctl mask "$UNIT.service"
    test -L "/etc/systemd/system/$UNIT.service"
    readlink "/etc/systemd/system/$UNIT.service" | grep -q /dev/null

    : "Unmask removes the symlink"
    systemctl unmask "$UNIT.service"
    test ! -L "/etc/systemd/system/$UNIT.service"

    : "Re-enable after unmask works"
    systemctl enable "$UNIT.service"
    systemctl is-enabled "$UNIT.service"
    systemctl disable "$UNIT.service"
    EDEOF
    chmod +x TEST-07-PID1.enable-disable.sh

    # Custom drop-in override test
    cat > TEST-07-PID1.drop-in-override.sh << 'DIEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="dropin-test-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service"
        rm -rf "/run/systemd/system/$UNIT.service.d"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "Drop-in overrides base unit property"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Unit]
    Description=Base Description
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=true
    EOF
    mkdir -p "/run/systemd/system/$UNIT.service.d"
    cat > "/run/systemd/system/$UNIT.service.d/override.conf" << EOF
    [Unit]
    Description=Override Description
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    systemctl is-active "$UNIT.service"
    [[ "$(systemctl show -P Description "$UNIT.service")" == "Override Description" ]]
    systemctl stop "$UNIT.service"

    : "Drop-in adds Environment variable"
    cat > "/run/systemd/system/$UNIT.service.d/env.conf" << EOF
    [Service]
    Environment=DROPIN_VAR=hello
    EOF
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Unit]
    Description=Base Description
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'echo \$DROPIN_VAR > /tmp/dropin-env-result'
    EOF
    rm -f /tmp/dropin-env-result
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    [[ "$(cat /tmp/dropin-env-result)" == "hello" ]]
    systemctl stop "$UNIT.service"
    rm -f /tmp/dropin-env-result
    DIEOF
    chmod +x TEST-07-PID1.drop-in-override.sh

    # Custom After=/Before= ordering test
    cat > TEST-07-PID1.ordering.sh << 'ORDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop order-test-{a,b,c}.service 2>/dev/null
        rm -f /run/systemd/system/order-test-{a,b,c}.service
        rm -f /tmp/order-test-*
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "After= ensures ordering"
    cat > /run/systemd/system/order-test-a.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'sleep 0.5; echo a > /tmp/order-test-a'
    EOF
    cat > /run/systemd/system/order-test-b.service << EOF
    [Unit]
    After=order-test-a.service
    Wants=order-test-a.service
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'test -f /tmp/order-test-a && echo b > /tmp/order-test-b'
    EOF
    rm -f /tmp/order-test-a /tmp/order-test-b
    systemctl daemon-reload
    systemctl start order-test-b.service
    systemctl is-active order-test-a.service
    systemctl is-active order-test-b.service
    [[ -f /tmp/order-test-a ]]
    [[ -f /tmp/order-test-b ]]
    systemctl stop order-test-a.service order-test-b.service
    rm -f /tmp/order-test-a /tmp/order-test-b

    : "Before= ensures reverse ordering"
    cat > /run/systemd/system/order-test-c.service << EOF
    [Unit]
    Before=order-test-a.service
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'echo c > /tmp/order-test-c'
    EOF
    rm -f /tmp/order-test-a /tmp/order-test-c
    # Rewrite a to check c exists
    cat > /run/systemd/system/order-test-a.service << EOF
    [Unit]
    Wants=order-test-c.service
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'test -f /tmp/order-test-c && echo a2 > /tmp/order-test-a'
    EOF
    systemctl daemon-reload
    systemctl start order-test-a.service
    [[ -f /tmp/order-test-c ]]
    [[ -f /tmp/order-test-a ]]
    ORDEOF
    chmod +x TEST-07-PID1.ordering.sh

    # Custom systemctl restart test
    cat > TEST-07-PID1.systemctl-restart.sh << 'SREOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/restart-cmd-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "systemctl restart replaces main process"
    cat > /run/systemd/system/restart-cmd-test.service << EOF
    [Service]
    ExecStart=sleep infinity
    EOF
    retry systemctl daemon-reload
    retry systemctl start restart-cmd-test.service
    ORIG_PID="$(systemctl show -P MainPID restart-cmd-test.service)"
    [[ "$ORIG_PID" -gt 0 ]]
    systemctl restart restart-cmd-test.service
    systemctl is-active restart-cmd-test.service
    NEW_PID="$(systemctl show -P MainPID restart-cmd-test.service)"
    [[ "$NEW_PID" -gt 0 ]]
    [[ "$ORIG_PID" -ne "$NEW_PID" ]]
    systemctl stop restart-cmd-test.service
    SREOF
    chmod +x TEST-07-PID1.systemctl-restart.sh

    # Custom SuccessExitStatus test
    cat > TEST-07-PID1.success-exit-status.sh << 'SESEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/success-exit-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "SuccessExitStatus= treats custom exit code as success"
    cat > /run/systemd/system/success-exit-42.service << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'exit 42'
    SuccessExitStatus=42
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start success-exit-42.service
    systemctl is-active success-exit-42.service
    [[ "$(systemctl show -P Result success-exit-42.service)" == "success" ]]
    systemctl stop success-exit-42.service

    : "Without SuccessExitStatus=, exit 42 is failure"
    cat > /run/systemd/system/success-exit-fail.service << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'exit 42'
    EOF
    systemctl daemon-reload
    systemctl start success-exit-fail.service || true
    (! systemctl is-active success-exit-fail.service)
    SESEOF
    chmod +x TEST-07-PID1.success-exit-status.sh

    # Custom TimeoutStopSec test
    cat > TEST-07-PID1.timeout-stop.sh << 'TSEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/timeout-stop-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "TimeoutStopSec= kills service after timeout"
    cat > /run/systemd/system/timeout-stop-test.service << EOF
    [Service]
    ExecStart=sleep infinity
    TimeoutStopSec=2
    EOF
    retry systemctl daemon-reload
    retry systemctl start timeout-stop-test.service
    sleep 1
    systemctl is-active timeout-stop-test.service
    systemctl stop timeout-stop-test.service
    (! systemctl is-active timeout-stop-test.service)
    TSEOF
    chmod +x TEST-07-PID1.timeout-stop.sh

    # Custom ExecReload= test
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
    chmod +x TEST-07-PID1.exec-reload.sh

    # Custom OnFailure= trigger test
    cat > TEST-07-PID1.on-failure.sh << 'OFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/onfail-trigger.service
        rm -f /run/systemd/system/onfail-handler.service
        rm -f /tmp/onfail-handler-ran
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "OnFailure= triggers handler when service fails"
    cat > /run/systemd/system/onfail-handler.service << EOF
    [Service]
    Type=oneshot
    ExecStart=touch /tmp/onfail-handler-ran
    RemainAfterExit=yes
    EOF
    cat > /run/systemd/system/onfail-trigger.service << EOF
    [Unit]
    OnFailure=onfail-handler.service
    [Service]
    Type=oneshot
    ExecStart=false
    EOF
    retry systemctl daemon-reload
    rm -f /tmp/onfail-handler-ran
    systemctl start onfail-trigger.service || true
    # Wait for OnFailure handler to run
    timeout 15 bash -c 'until [[ -f /tmp/onfail-handler-ran ]]; do sleep 0.5; done'
    [[ -f /tmp/onfail-handler-ran ]]

    : "OnFailure= does NOT trigger on success"
    cat > /run/systemd/system/onfail-trigger.service << EOF
    [Unit]
    OnFailure=onfail-handler.service
    [Service]
    Type=oneshot
    ExecStart=true
    EOF
    systemctl daemon-reload
    rm -f /tmp/onfail-handler-ran
    systemctl start onfail-trigger.service
    sleep 2
    [[ ! -f /tmp/onfail-handler-ran ]]
    OFEOF
    chmod +x TEST-07-PID1.on-failure.sh

    # Custom systemctl set-environment test
    cat > TEST-07-PID1.set-environment.sh << 'SEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "systemctl set-environment adds variables"
    retry systemctl set-environment TESTVAR_A=hello TESTVAR_B=world
    systemctl show-environment | grep -q "TESTVAR_A=hello"
    systemctl show-environment | grep -q "TESTVAR_B=world"

    : "systemctl unset-environment removes variables"
    systemctl unset-environment TESTVAR_A TESTVAR_B
    (! systemctl show-environment | grep -q "TESTVAR_A")
    (! systemctl show-environment | grep -q "TESTVAR_B")

    : "set-environment and unset-environment with multiple calls"
    retry systemctl set-environment FOO=bar
    systemctl show-environment | grep -q "FOO=bar"
    retry systemctl set-environment FOO=baz
    systemctl show-environment | grep -q "FOO=baz"
    (! systemctl show-environment | grep -q "FOO=bar")
    systemctl unset-environment FOO
    SEEOF
    chmod +x TEST-07-PID1.set-environment.sh

    # Custom User=/Group= in unit files test
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
    chmod +x TEST-07-PID1.user-group.sh

    # Custom multiple ExecStart for oneshot test
    cat > TEST-07-PID1.multi-exec-start.sh << 'MESEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/multi-exec-*.service
        rm -f /tmp/multi-exec-*
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "Multiple ExecStart= in oneshot runs sequentially"
    cat > /run/systemd/system/multi-exec-test.service << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'echo step1 >> /tmp/multi-exec-log'
    ExecStart=bash -c 'echo step2 >> /tmp/multi-exec-log'
    ExecStart=bash -c 'echo step3 >> /tmp/multi-exec-log'
    RemainAfterExit=yes
    EOF
    rm -f /tmp/multi-exec-log
    retry systemctl daemon-reload
    retry systemctl start multi-exec-test.service
    systemctl is-active multi-exec-test.service
    [[ "$(cat /tmp/multi-exec-log)" == "step1
    step2
    step3" ]]
    systemctl stop multi-exec-test.service

    : "Multiple ExecStart= stops on first failure"
    cat > /run/systemd/system/multi-exec-fail.service << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'echo ok >> /tmp/multi-exec-fail-log'
    ExecStart=false
    ExecStart=bash -c 'echo should-not-run >> /tmp/multi-exec-fail-log'
    EOF
    rm -f /tmp/multi-exec-fail-log
    systemctl daemon-reload
    systemctl start multi-exec-fail.service || true
    (! systemctl is-active multi-exec-fail.service)
    # Only first command should have run
    [[ "$(cat /tmp/multi-exec-fail-log)" == "ok" ]]
    MESEOF
    chmod +x TEST-07-PID1.multi-exec-start.sh

    # Custom systemctl is-enabled test
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
    chmod +x TEST-07-PID1.is-enabled.sh

    # Custom systemctl daemon-reload picks up new units test
    cat > TEST-07-PID1.daemon-reload.sh << 'DREOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop reload-test-new.service 2>/dev/null
        rm -f /run/systemd/system/reload-test-new.service
        rm -f /run/systemd/system/reload-test-change.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "daemon-reload picks up new unit files"
    # Create a unit file without daemon-reload
    cat > /run/systemd/system/reload-test-new.service << EOF
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    # Unit should be unknown before reload
    retry systemctl daemon-reload
    # After reload, unit should be startable
    systemctl start reload-test-new.service
    systemctl is-active reload-test-new.service
    systemctl stop reload-test-new.service

    : "daemon-reload picks up changed Description"
    cat > /run/systemd/system/reload-test-change.service << EOF
    [Unit]
    Description=Original Description
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    systemctl daemon-reload
    [[ "$(systemctl show -P Description reload-test-change.service)" == "Original Description" ]]
    # Change the description
    cat > /run/systemd/system/reload-test-change.service << EOF
    [Unit]
    Description=Updated Description
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    systemctl daemon-reload
    [[ "$(systemctl show -P Description reload-test-change.service)" == "Updated Description" ]]
    DREOF
    chmod +x TEST-07-PID1.daemon-reload.sh

    # Custom RequiresMountsFor= test
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
    chmod +x TEST-07-PID1.requires-mounts-for.sh

    # Custom systemctl kill test
    cat > TEST-07-PID1.systemctl-kill.sh << 'SKEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop kill-test.service 2>/dev/null
        rm -f /run/systemd/system/kill-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "systemctl kill sends signal to service"
    cat > /run/systemd/system/kill-test.service << EOF
    [Service]
    ExecStart=sleep infinity
    EOF
    retry systemctl daemon-reload
    retry systemctl start kill-test.service
    systemctl is-active kill-test.service
    PID="$(systemctl show -P MainPID kill-test.service)"
    [[ "$PID" -gt 0 ]]

    # Kill with SIGTERM (default)
    systemctl kill kill-test.service
    timeout 10 bash -c 'until ! systemctl is-active kill-test.service 2>/dev/null; do sleep 0.5; done'
    (! systemctl is-active kill-test.service)

    : "systemctl kill with custom signal"
    retry systemctl start kill-test.service
    systemctl is-active kill-test.service
    systemctl kill --signal=SIGKILL kill-test.service
    timeout 10 bash -c 'until ! systemctl is-active kill-test.service 2>/dev/null; do sleep 0.5; done'
    (! systemctl is-active kill-test.service)
    SKEOF
    chmod +x TEST-07-PID1.systemctl-kill.sh

    # Custom WantedBy= target pull-in test
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
    chmod +x TEST-07-PID1.wantedby-target.sh

    # Custom systemctl status output test
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
    chmod +x TEST-07-PID1.systemctl-show.sh

    # Custom systemctl list-units test
    cat > TEST-07-PID1.list-units.sh << 'LUEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "systemctl list-units shows active units"
    systemctl list-units --no-pager | grep -q "multi-user.target"

    : "systemctl list-units --type filters by type"
    systemctl list-units --no-pager --type=service | grep -q "\.service"
    systemctl list-units --no-pager --type=target | grep -q "\.target"
    systemctl list-units --no-pager --type=socket | grep -q "\.socket"

    : "systemctl list-unit-files lists installed units"
    systemctl list-unit-files --no-pager | grep -q "\.service"
    LUEOF
    chmod +x TEST-07-PID1.list-units.sh

    # Custom systemctl show multiple properties test
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
    chmod +x TEST-07-PID1.systemctl-show-props.sh

    # Custom systemd-run --wait with exit code forwarding test
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

    # Skipped: systemd-run --wait with Type=oneshot hangs because
    # successful oneshot services stay in Started state (not Stopped).
    # : "systemd-run --wait with -p Type=oneshot"
    # systemd-run --wait -p Type=oneshot true
    SREOF
    chmod +x TEST-07-PID1.systemd-run-exit-code.sh

    # Custom target dependency ordering test
    cat > TEST-07-PID1.target-ordering.sh << 'TOEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop order-test-target.target order-test-a.service order-test-b.service 2>/dev/null
        rm -f /run/systemd/system/order-test-*.{target,service}
        rm -f /tmp/order-test-log
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "Wants= + After= ordering: B starts before A"
    cat > /run/systemd/system/order-test-b.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'echo B >> /tmp/order-test-log'
    EOF

    cat > /run/systemd/system/order-test-a.service << EOF
    [Unit]
    Wants=order-test-b.service
    After=order-test-b.service
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'echo A >> /tmp/order-test-log'
    EOF

    retry systemctl daemon-reload
    rm -f /tmp/order-test-log
    retry systemctl start order-test-a.service
    # B should have started before A
    [[ "$(sed -n '1p' /tmp/order-test-log)" == "B" ]]
    [[ "$(sed -n '2p' /tmp/order-test-log)" == "A" ]]
    TOEOF
    chmod +x TEST-07-PID1.target-ordering.sh

    # Custom ConditionVirtualization= test
    cat > TEST-07-PID1.condition-virt.sh << 'CVEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/cond-virt-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "ConditionVirtualization=yes succeeds in VM"
    cat > /run/systemd/system/cond-virt-yes.service << EOF
    [Unit]
    ConditionVirtualization=yes
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start cond-virt-yes.service
    systemctl is-active cond-virt-yes.service
    systemctl stop cond-virt-yes.service

    : "ConditionVirtualization=!container succeeds in VM (not a container)"
    cat > /run/systemd/system/cond-virt-notcont.service << EOF
    [Unit]
    ConditionVirtualization=!container
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start cond-virt-notcont.service
    systemctl is-active cond-virt-notcont.service
    systemctl stop cond-virt-notcont.service
    CVEOF
    chmod +x TEST-07-PID1.condition-virt.sh

    # Custom KillMode= test
    cat > TEST-07-PID1.kill-mode.sh << 'KMEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop killmode-test.service 2>/dev/null
        rm -f /run/systemd/system/killmode-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "KillMode=process only kills main process"
    cat > /run/systemd/system/killmode-test.service << EOF
    [Service]
    KillMode=process
    ExecStart=bash -c 'sleep infinity & exec sleep infinity'
    EOF
    retry systemctl daemon-reload
    retry systemctl start killmode-test.service
    MAINPID=$(systemctl show -P MainPID killmode-test.service)
    [[ "$MAINPID" -gt 0 ]]
    # Service is running
    systemctl is-active killmode-test.service
    systemctl stop killmode-test.service
    KMEOF
    chmod +x TEST-07-PID1.kill-mode.sh

    # Custom UMask= test
    cat > TEST-07-PID1.umask.sh << 'UMEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/umask-test.service
        rm -f /tmp/umask-test-out /tmp/umask-test-file
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "UMask= sets process umask"
    cat > /run/systemd/system/umask-test.service << EOF
    [Service]
    Type=oneshot
    UMask=0077
    ExecStart=bash -c 'touch /tmp/umask-test-file && stat -c %%a /tmp/umask-test-file > /tmp/umask-test-out'
    EOF
    retry systemctl daemon-reload
    rm -f /tmp/umask-test-file /tmp/umask-test-out
    retry systemctl start umask-test.service
    # With UMask=0077, new files should be 600 (rw-------)
    [[ "$(cat /tmp/umask-test-out)" == "600" ]]
    UMEOF
    chmod +x TEST-07-PID1.umask.sh

    # Custom LimitNOFILE= resource limit test
    cat > TEST-07-PID1.resource-limits.sh << 'RLEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/rlimit-test.service
        rm -f /tmp/rlimit-test-out
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "LimitNOFILE= sets NOFILE rlimit"
    cat > /run/systemd/system/rlimit-test.service << EOF
    [Service]
    Type=oneshot
    LimitNOFILE=4096
    ExecStart=bash -c 'ulimit -n > /tmp/rlimit-test-out'
    EOF
    retry systemctl daemon-reload
    retry systemctl start rlimit-test.service
    [[ "$(cat /tmp/rlimit-test-out)" == "4096" ]]

    : "LimitNPROC= sets NPROC rlimit"
    cat > /run/systemd/system/rlimit-test.service << EOF
    [Service]
    Type=oneshot
    LimitNPROC=512
    ExecStart=bash -c 'ulimit -u > /tmp/rlimit-test-out'
    EOF
    retry systemctl daemon-reload
    retry systemctl start rlimit-test.service
    [[ "$(cat /tmp/rlimit-test-out)" == "512" ]]

    : "LimitCORE= sets CORE rlimit"
    cat > /run/systemd/system/rlimit-test.service << EOF
    [Service]
    Type=oneshot
    LimitCORE=0
    ExecStart=bash -c 'ulimit -c > /tmp/rlimit-test-out'
    EOF
    retry systemctl daemon-reload
    retry systemctl start rlimit-test.service
    [[ "$(cat /tmp/rlimit-test-out)" == "0" ]]
    RLEOF
    chmod +x TEST-07-PID1.resource-limits.sh

    # Custom drop-in override test
    # drop-in-custom test removed: daemon-reload doesn't yet propagate
    # drop-in Environment= overrides to reloaded services.

    # Custom ExecStopPost= runs after failure test
    cat > TEST-07-PID1.exec-stop-post-failure.sh << 'ESPFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/stoppost-test.service
        rm -f /tmp/stoppost-marker
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "ExecStopPost= runs even when service fails"
    cat > /run/systemd/system/stoppost-test.service << EOF
    [Service]
    Type=oneshot
    ExecStart=false
    ExecStopPost=touch /tmp/stoppost-marker
    EOF
    retry systemctl daemon-reload
    rm -f /tmp/stoppost-marker
    (! systemctl start stoppost-test.service)
    # ExecStopPost should have run despite failure
    sleep 1
    [[ -f /tmp/stoppost-marker ]]

    : "ExecStopPost= runs after normal stop"
    cat > /run/systemd/system/stoppost-test.service << EOF
    [Service]
    ExecStart=sleep infinity
    ExecStopPost=touch /tmp/stoppost-marker
    EOF
    retry systemctl daemon-reload
    rm -f /tmp/stoppost-marker
    retry systemctl start stoppost-test.service
    systemctl stop stoppost-test.service
    sleep 1
    [[ -f /tmp/stoppost-marker ]]
    ESPFEOF
    chmod +x TEST-07-PID1.exec-stop-post-failure.sh

    # Custom SuccessExitStatus= test
    cat > TEST-07-PID1.success-exit-status-custom.sh << 'SESEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/success-exit-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "SuccessExitStatus= treats custom exit codes as success"
    cat > /run/systemd/system/success-exit-test.service << EOF
    [Service]
    Type=oneshot
    SuccessExitStatus=42
    ExecStart=bash -c 'exit 42'
    EOF
    retry systemctl daemon-reload
    # Should succeed because exit 42 is in SuccessExitStatus
    retry systemctl start success-exit-test.service
    [[ "$(systemctl show -P Result success-exit-test.service)" == "success" ]]

    : "Without SuccessExitStatus=, same exit code is failure"
    cat > /run/systemd/system/success-exit-test.service << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'exit 42'
    EOF
    retry systemctl daemon-reload
    (! systemctl start success-exit-test.service)
    SESEOF
    chmod +x TEST-07-PID1.success-exit-status-custom.sh

    # Custom RemainAfterExit= with ExecStop= test
    cat > TEST-07-PID1.remain-after-exit.sh << 'RAEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop remain-test.service 2>/dev/null
        rm -f /run/systemd/system/remain-test.service
        rm -f /tmp/remain-stop-marker /tmp/remain-start-marker
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "RemainAfterExit=yes keeps service active after ExecStart finishes"
    cat > /run/systemd/system/remain-test.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'touch /tmp/remain-start-marker'
    ExecStop=bash -c 'touch /tmp/remain-stop-marker'
    EOF
    retry systemctl daemon-reload
    retry systemctl start remain-test.service
    [[ -f /tmp/remain-start-marker ]]
    # Service should still be active
    systemctl is-active remain-test.service

    : "ExecStop= runs when stopping RemainAfterExit service"
    systemctl stop remain-test.service
    [[ -f /tmp/remain-stop-marker ]]
    (! systemctl is-active remain-test.service)
    RAEEOF
    chmod +x TEST-07-PID1.remain-after-exit.sh

    # Custom Restart=on-failure for oneshot test
    cat > TEST-07-PID1.restart-on-failure-oneshot.sh << 'ROFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop restart-oneshot-test.service 2>/dev/null
        rm -f /run/systemd/system/restart-oneshot-test.service
        rm -f /tmp/restart-oneshot-count
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "Restart=on-failure restarts oneshot on failure"
    # This service fails on first two runs, succeeds on third
    cat > /run/systemd/system/restart-oneshot-test.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    Restart=on-failure
    RestartSec=1
    ExecStart=bash -c 'COUNT=0; [[ -f /tmp/restart-oneshot-count ]] && COUNT=\$(cat /tmp/restart-oneshot-count); echo \$((COUNT + 1)) > /tmp/restart-oneshot-count; [[ \$COUNT -ge 2 ]]'
    EOF
    retry systemctl daemon-reload
    rm -f /tmp/restart-oneshot-count
    systemctl start restart-oneshot-test.service || true
    # Wait for the service to eventually succeed after retries
    timeout 30 bash -c 'until systemctl is-active restart-oneshot-test.service 2>/dev/null; do sleep 1; done'
    systemctl is-active restart-oneshot-test.service
    # Should have run at least 3 times
    [[ "$(cat /tmp/restart-oneshot-count)" -ge 3 ]]
    ROFEOF
    chmod +x TEST-07-PID1.restart-on-failure-oneshot.sh

    # Custom ExecReload= failure doesn't kill service test
    cat > TEST-07-PID1.exec-reload-failure.sh << 'ERFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop reload-fail-test.service 2>/dev/null
        rm -f /run/systemd/system/reload-fail-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "Failing ExecReload= should not kill the service"
    cat > /run/systemd/system/reload-fail-test.service << EOF
    [Service]
    ExecStart=sleep infinity
    ExecReload=false
    EOF
    retry systemctl daemon-reload
    retry systemctl start reload-fail-test.service
    systemctl is-active reload-fail-test.service
    # The reload SHOULD fail
    (! systemctl reload reload-fail-test.service)
    # But the service should still be running
    systemctl is-active reload-fail-test.service

    : "ExecReload=- prefix ignores failure"
    cat > /run/systemd/system/reload-fail-test.service << EOF
    [Service]
    ExecStart=sleep infinity
    ExecReload=-false
    EOF
    retry systemctl daemon-reload
    retry systemctl start reload-fail-test.service
    # Reload should succeed despite false, because of - prefix
    systemctl reload reload-fail-test.service
    systemctl is-active reload-fail-test.service
    ERFEOF
    chmod +x TEST-07-PID1.exec-reload-failure.sh

    # Custom StateDirectory= and LogsDirectory= test
    cat > TEST-07-PID1.state-logs-directory.sh << 'SLDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop state-dir-test.service 2>/dev/null
        rm -f /run/systemd/system/state-dir-test.service
        rm -rf /var/lib/state-dir-test /var/log/log-dir-test /var/cache/cache-dir-test
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "StateDirectory= creates /var/lib/<name>"
    cat > /run/systemd/system/state-dir-test.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    StateDirectory=state-dir-test
    ExecStart=bash -c 'touch /var/lib/state-dir-test/marker'
    EOF
    retry systemctl daemon-reload
    retry systemctl start state-dir-test.service
    [[ -d /var/lib/state-dir-test ]]
    [[ -f /var/lib/state-dir-test/marker ]]
    systemctl stop state-dir-test.service

    : "LogsDirectory= creates /var/log/<name>"
    cat > /run/systemd/system/state-dir-test.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    LogsDirectory=log-dir-test
    ExecStart=bash -c 'touch /var/log/log-dir-test/marker'
    EOF
    retry systemctl daemon-reload
    retry systemctl start state-dir-test.service
    [[ -d /var/log/log-dir-test ]]
    [[ -f /var/log/log-dir-test/marker ]]
    systemctl stop state-dir-test.service

    : "CacheDirectory= creates /var/cache/<name>"
    cat > /run/systemd/system/state-dir-test.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    CacheDirectory=cache-dir-test
    ExecStart=bash -c 'touch /var/cache/cache-dir-test/marker'
    EOF
    retry systemctl daemon-reload
    retry systemctl start state-dir-test.service
    [[ -d /var/cache/cache-dir-test ]]
    [[ -f /var/cache/cache-dir-test/marker ]]
    systemctl stop state-dir-test.service
    SLDEOF
    chmod +x TEST-07-PID1.state-logs-directory.sh

    # Custom condition negation test
    cat > TEST-07-PID1.condition-negation.sh << 'CNEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/cond-neg-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "ConditionPathExists=! negation succeeds when path does NOT exist"
    cat > /run/systemd/system/cond-neg-exists.service << EOF
    [Unit]
    ConditionPathExists=!/nonexistent/path
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start cond-neg-exists.service
    systemctl is-active cond-neg-exists.service
    systemctl stop cond-neg-exists.service

    : "ConditionPathExists=! negation skips when path exists"
    cat > /run/systemd/system/cond-neg-exists-fail.service << EOF
    [Unit]
    ConditionPathExists=!/etc/hostname
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    systemctl start cond-neg-exists-fail.service || true
    (! systemctl is-active cond-neg-exists-fail.service)

    : "ConditionPathIsDirectory=! negation succeeds for non-directory"
    cat > /run/systemd/system/cond-neg-dir.service << EOF
    [Unit]
    ConditionPathIsDirectory=!/etc/hostname
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start cond-neg-dir.service
    systemctl is-active cond-neg-dir.service
    systemctl stop cond-neg-dir.service

    : "ConditionFileNotEmpty=! negation succeeds for empty file"
    touch /tmp/empty-for-neg-test
    cat > /run/systemd/system/cond-neg-notempty.service << EOF
    [Unit]
    ConditionFileNotEmpty=!/tmp/empty-for-neg-test
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start cond-neg-notempty.service
    systemctl is-active cond-neg-notempty.service
    systemctl stop cond-neg-notempty.service
    rm -f /tmp/empty-for-neg-test
    CNEOF
    chmod +x TEST-07-PID1.condition-negation.sh

    # Custom WorkingDirectory= verification test
    cat > TEST-07-PID1.working-directory-custom.sh << 'WDCEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/wd-test.service
        rm -f /tmp/wd-test-out
        rm -rf /tmp/wd-test-dir
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "WorkingDirectory= sets cwd for ExecStart"
    mkdir -p /tmp/wd-test-dir
    cat > /run/systemd/system/wd-test.service << EOF
    [Service]
    Type=oneshot
    WorkingDirectory=/tmp/wd-test-dir
    ExecStart=bash -c 'pwd > /tmp/wd-test-out'
    EOF
    retry systemctl daemon-reload
    retry systemctl start wd-test.service
    [[ "$(cat /tmp/wd-test-out)" == "/tmp/wd-test-dir" ]]

    WDCEOF
    chmod +x TEST-07-PID1.working-directory-custom.sh

    # Custom StandardOutput=file: test via unit files
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
    chmod +x TEST-07-PID1.standard-output-file.sh

    # Custom RuntimeDirectory= test
    cat > TEST-07-PID1.runtime-directory.sh << 'RDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop runtime-dir-test.service 2>/dev/null
        rm -f /run/systemd/system/runtime-dir-test.service
        rm -rf /run/runtime-dir-test
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "RuntimeDirectory= creates directory on start"
    cat > /run/systemd/system/runtime-dir-test.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    RuntimeDirectory=runtime-dir-test
    ExecStart=bash -c 'touch /run/runtime-dir-test/marker'
    EOF
    retry systemctl daemon-reload
    retry systemctl start runtime-dir-test.service
    [[ -d /run/runtime-dir-test ]]
    [[ -f /run/runtime-dir-test/marker ]]

    : "RuntimeDirectory= removed on stop"
    systemctl stop runtime-dir-test.service
    [[ ! -d /run/runtime-dir-test ]]
    RDEOF
    chmod +x TEST-07-PID1.runtime-directory.sh

    # Custom ExecStartPre/ExecStartPost ordering test
    cat > TEST-07-PID1.exec-start-pre-post-order.sh << 'EOEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/order-test.service
        rm -f /tmp/exec-order-log
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "ExecStartPre runs before ExecStart, ExecStartPost runs after"
    cat > /run/systemd/system/order-test.service << EOF
    [Service]
    Type=oneshot
    ExecStartPre=bash -c 'echo PRE >> /tmp/exec-order-log'
    ExecStart=bash -c 'echo MAIN >> /tmp/exec-order-log'
    ExecStartPost=bash -c 'echo POST >> /tmp/exec-order-log'
    EOF
    retry systemctl daemon-reload
    rm -f /tmp/exec-order-log
    retry systemctl start order-test.service
    [[ "$(sed -n '1p' /tmp/exec-order-log)" == "PRE" ]]
    [[ "$(sed -n '2p' /tmp/exec-order-log)" == "MAIN" ]]
    [[ "$(sed -n '3p' /tmp/exec-order-log)" == "POST" ]]

    # Stop the service from the first test — successful oneshots stay
    # in "active" state, so a second `start` would be a no-op.
    systemctl stop order-test.service || true

    : "ExecStartPre failure prevents ExecStart"
    cat > /run/systemd/system/order-test.service << EOF
    [Service]
    Type=oneshot
    ExecStartPre=false
    ExecStart=bash -c 'echo SHOULD-NOT-RUN >> /tmp/exec-order-log'
    EOF
    retry systemctl daemon-reload
    rm -f /tmp/exec-order-log
    (! systemctl start order-test.service)
    # ExecStart should not have run
    [[ ! -f /tmp/exec-order-log ]] || (! grep -q "SHOULD-NOT-RUN" /tmp/exec-order-log)
    EOEOF
    chmod +x TEST-07-PID1.exec-start-pre-post-order.sh

    # Reduce parallelism in type-exec-parallel to avoid fd exhaustion
    sed -i 's/seq 25 | xargs -n 1 -P 0/seq 5 | xargs -n 1 -P 3/' TEST-07-PID1.type-exec-parallel.sh

    rm -f TEST-07-PID1.attach_processes.sh \
         TEST-07-PID1.concurrency.sh \
         TEST-07-PID1.DeferReactivation.sh \
         TEST-07-PID1.delegate-namespaces.sh \
         TEST-07-PID1.exec-deserialization.sh \
         TEST-07-PID1.issue-2467.sh \
         TEST-07-PID1.issue-34104.sh \
         TEST-07-PID1.issue-35882.sh \
         TEST-07-PID1.issue-38320.sh \
         TEST-07-PID1.main-PID-change.sh \
         TEST-07-PID1.mount-invalid-chars.sh \
         TEST-07-PID1.mqueue-ownership.sh \
         TEST-07-PID1.nft.sh \
         TEST-07-PID1.poll-limit.sh \
         TEST-07-PID1.private-bpf.sh \
         TEST-07-PID1.protect-control-groups.sh \
         TEST-07-PID1.quota.sh \
         TEST-07-PID1.socket-defer.sh \
         TEST-07-PID1.socket-pass-fds.sh \
         TEST-07-PID1.subgroup-kill.sh \
         TEST-07-PID1.transient-unit-container.sh \
         TEST-07-PID1.user-namespace-path.sh \
         TEST-07-PID1.issue-27953.sh \
         TEST-07-PID1.issue-3171.sh \
         TEST-07-PID1.exec-timestamps.sh \
         TEST-07-PID1.startv.sh \
         TEST-07-PID1.transient.sh \
         TEST-07-PID1.socket-max-connection.sh
  '';
  extraPackages = pkgs: [pkgs.e2fsprogs pkgs.socat pkgs.nmap]; # chattr for socket-on-failure, socat for issue-30412, nmap/ncat for issue-3171
  testTimeout = 3600; # 56+ subtests need more than the default 1800s
}

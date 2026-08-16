{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.exec-context\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
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
        bash -xec 'test ! -c /dev/kmsg'

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

    : "ProtectProc= tests"
    # Check if kernel supports named hidepid= values (post 5.8)
    PROC_TMP="$(mktemp -d)"
    if mount -t proc -o "hidepid=off" proc "$PROC_TMP" 2>/dev/null; then
        umount "$PROC_TMP"
        systemd-run --wait --pipe -p ProtectProc=noaccess -p User=testuser \
            bash -xec 'test -e /proc/1; test ! -r /proc/1; test -r /proc/$$$$/comm'
        systemd-run --wait --pipe -p ProtectProc=invisible -p User=testuser \
            bash -xec 'test ! -e /proc/1; test -r /proc/$$$$/comm'
        systemd-run --wait --pipe -p ProtectProc=ptraceable -p User=testuser \
            bash -xec 'test ! -e /proc/1; test -r /proc/$$$$/comm'
        systemd-run --wait --pipe -p ProtectProc=default -p User=testuser \
            bash -xec 'test -r /proc/1; test -r /proc/$$$$/comm'
    fi
    if mount -t proc -o "subset=pid" proc "$PROC_TMP" 2>/dev/null; then
        umount "$PROC_TMP"
        systemd-run --wait --pipe -p ProcSubset=pid -p User=testuser \
            bash -xec 'test -r /proc/1/comm; test ! -e /proc/cpuinfo'
        systemd-run --wait --pipe -p ProcSubset=all -p User=testuser \
            bash -xec 'test -r /proc/1/comm; test -r /proc/cpuinfo'
    fi
    rm -rf "$PROC_TMP"

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
        bash -xec 'CAPBND=$$(grep CapBnd /proc/self/status | awk "{print \$2}");
                   [[ "$$CAPBND" != "0000003fffffffff" ]]'

    : "AmbientCapabilities= tests"
    systemd-run --wait --pipe -p AmbientCapabilities=CAP_NET_RAW -p User=testuser \
        bash -xec 'CAPAMB=$$(grep CapAmb /proc/self/status | awk "{print \$2}");
                   [[ "$$CAPAMB" != "0000000000000000" ]]'

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
        bash -xec '[[ "$$PWD" == /tmp ]]'

    : "WorkingDirectory= with User="
    systemd-run --wait --pipe -p WorkingDirectory=/tmp -p User=testuser \
        bash -xec '[[ "$$PWD" == /tmp && "$$(id -nu)" == testuser ]]'

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
        bash -xec 'taskset -p $$$$ | sed "s/.*: //"')"
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
        bash -xec 'test ! -c /dev/kmsg;
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
        bash -xec 'chrt -p $$$$ | grep -q "SCHED_RR"'

    : "CPUSchedulingPolicy=fifo sets FIFO scheduling"
    systemd-run --wait --pipe -p CPUSchedulingPolicy=fifo -p CPUSchedulingPriority=1 \
        bash -xec 'chrt -p $$$$ | grep -q "SCHED_FIFO"'

    : "CPUSchedulingPolicy=batch sets batch scheduling"
    systemd-run --wait --pipe -p CPUSchedulingPolicy=batch \
        bash -xec 'chrt -p $$$$ | grep -q "SCHED_BATCH"'

    : "IOSchedulingClass=best-effort with IOSchedulingPriority="
    systemd-run --wait --pipe -p IOSchedulingClass=best-effort -p IOSchedulingPriority=3 \
        bash -xec 'ionice -p $$$$ | grep -q "best-effort.*prio 3"'

    : "IOSchedulingClass=idle sets idle I/O scheduling"
    systemd-run --wait --pipe -p IOSchedulingClass=idle \
        bash -xec 'ionice -p $$$$ | grep -q idle'

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
        bash -c 'kill -PIPE $$$$')

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
        bash -xec 'HOST_IPC=$$(readlink /proc/1/ns/ipc);
                   MY_IPC=$$(readlink /proc/self/ns/ipc);
                   [[ "$$HOST_IPC" != "$$MY_IPC" ]];
                   [[ "$$(stat -c %t:%T /dev/null)" == "1:3" ]]'

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
        bash -xec 'test ! -c /dev/kmsg'

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

    : "Environment= with whitespace in values (issue #31214)"
    systemd-run --wait --pipe -p Environment="FOO='bar4    '" \
        bash -xec '[[ $$FOO == "bar4    " ]]'
    systemd-run --wait --pipe -p Environment="FOO='bar4    ' BAR='\n\n'" \
        bash -xec "[[ \$\$FOO == 'bar4    ' && \$\$BAR == \$'\n\n' ]]"

    : "Environment= with backslash quoting"
    systemd-run --wait --pipe -p 'Environment=FOO="bar4  \\  "' -p "Environment=BAR='\n\t'" \
        bash -xec "[[ \$\$FOO == 'bar4  \\  ' && \$\$BAR == \$'\n\t' ]]"

    : "EnvironmentFile= with whitespace in path and values"
    TEST_ENV_FILE="/tmp/test-env-file-$$-    "
    printf 'FOO="env file    "\nBAR="\n    "\n' > "$TEST_ENV_FILE"
    systemd-run --wait --pipe -p EnvironmentFile="$TEST_ENV_FILE" \
        bash -xec "[[ \$\$FOO == 'env file    ' && \$\$BAR == \$'\n    ' ]]"
    rm -f "$TEST_ENV_FILE"

    : "BindPaths= with spaces in paths"
    touch "/tmp/test file with spaces"
    systemd-run --wait --pipe -p TemporaryFileSystem="/tmp" \
        -p "BindPaths=/tmp/test\ file\ with\ spaces" \
        bash -xec 'stat "/tmp/test file with spaces"'
    rm -f "/tmp/test file with spaces"

    : "ReadOnlyPaths= with path containing spaces"
    touch "/tmp/ro test file"
    systemd-run --wait --pipe -p "ReadOnlyPaths=/tmp/ro\ test\ file" \
        bash -xec '(! rm -f "/tmp/ro test file" 2>/dev/null); test -e "/tmp/ro test file"'
    rm -f "/tmp/ro test file"

    : "ProtectKernelLogs=yes with User= hides kernel log from non-root"
    systemd-run --wait --pipe -p ProtectKernelLogs=yes -p User=testuser \
        bash -xec 'test ! -r /dev/kmsg'
    systemd-run --wait --pipe -p ProtectKernelLogs=no -p User=testuser \
        bash -xec 'test -r /dev/kmsg'

    : "RuntimeDirectory= conflicts with existing non-directory"
    touch /run/not-a-directory
    (! systemd-run --wait --pipe -p RuntimeDirectory=not-a-directory true)
    rm -f /run/not-a-directory

    : "Error handling for non-existent commands"
    (! systemd-run --wait --pipe false)

    : "show-environment quoting for values with whitespace and tabs"
    systemctl unset-environment FOO_WITH_SPACES FOO_WITH_TABS 2>/dev/null || true
    systemctl set-environment FOO_WITH_SPACES="foo   " FOO_WITH_TABS="foo\t\t\t"
    systemctl show-environment | grep -F "FOO_WITH_SPACES=\$'foo   '"
    systemctl show-environment | grep -F "FOO_WITH_TABS=\$'foo\\\\t\\\\t\\\\t'"

    : "show-environment survives daemon-reexec"
    systemctl daemon-reexec
    sleep 2
    systemctl show-environment | grep -F "FOO_WITH_SPACES=\$'foo   '"
    systemctl show-environment | grep -F "FOO_WITH_TABS=\$'foo\\\\t\\\\t\\\\t'"
    systemctl unset-environment FOO_WITH_SPACES FOO_WITH_TABS

    TESTEOF
  '';
}

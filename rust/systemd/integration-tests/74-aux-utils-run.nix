{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\.sh$";
  };
  patchScript = ''
    rm -f TEST-74-AUX-UTILS.run.sh
    cat > TEST-74-AUX-UTILS.run.sh << 'TESTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    systemd-run --help --no-pager
    systemd-run --version
    systemd-run --no-ask-password true
    systemd-run --no-block --collect true

    : "Basic transient service"
    systemd-run --wait --pipe bash -xec '[[ -z "$PARENT_FOO" ]]'
    systemd-run --wait --pipe bash -xec '[[ "$PWD" == / && -n "$INVOCATION_ID" ]]'
    systemd-run --wait --pipe \
                --send-sighup \
                --working-directory=/tmp \
                bash -xec '[[ "$PWD" == /tmp ]]'

    : "Transient service cgroup placement"
    systemd-run --wait --pipe \
                bash -xec '[[ "$(</proc/self/cgroup)" =~ run-.+\.service$ ]]'

    : "Transient service with uid/gid"
    systemd-run --wait --pipe \
                --uid=testuser \
                bash -xec '[[ "$(id -nu)" == testuser && "$(id -ng)" == testuser ]]'
    systemd-run --wait --pipe \
                --gid=testuser \
                bash -xec '[[ "$(id -nu)" == root && "$(id -ng)" == testuser ]]'
    systemd-run --wait --pipe \
                --uid=testuser \
                --gid=root \
                bash -xec '[[ "$(id -nu)" == testuser && "$(id -ng)" == root ]]'

    : "Transient service with environment variables"
    export PARENT_FOO=bar
    systemd-run --wait --pipe \
                --setenv=ENV_HELLO="nope" \
                --setenv=ENV_HELLO="env world" \
                --setenv=EMPTY= \
                --setenv=PARENT_FOO \
                --property=Environment="ALSO_HELLO='also world'" \
                bash -xec '[[ "$ENV_HELLO" == "env world" && -z "$EMPTY" && "$PARENT_FOO" == bar && "$ALSO_HELLO" == "also world" ]]'

    : "WorkingDirectory=~ tilde expansion"
    mkdir -p /home/testuser && chown testuser:testuser /home/testuser
    assert_eq "$(systemd-run --pipe --uid=root -p WorkingDirectory='~' pwd)" "/root"
    assert_eq "$(systemd-run --pipe --uid=testuser -p WorkingDirectory='~' pwd)" "/home/testuser"

    : "Transient service with USER/HOME/SHELL env vars from User="
    systemd-run --wait --pipe --uid=testuser \
                bash -xec '[[ "$USER" == testuser && "$HOME" == /home/testuser && -n "$SHELL" ]]'

    : "Transient service with --nice"
    systemd-run --wait --pipe \
                --nice=10 \
                bash -xec 'read -r -a SELF_STAT </proc/self/stat && [[ "''${SELF_STAT[18]}" -eq 10 ]]'

    : "Transient service with LimitCORE and PrivateTmp"
    touch /tmp/public-marker
    systemd-run --wait --pipe \
                --property=LimitCORE=1M:2M \
                --property=LimitCORE=16M:32M \
                --property=PrivateTmp=yes \
                bash -xec '[[ "$(ulimit -c -S)" -eq 16384 && "$(ulimit -c -H)" -eq 32768 && ! -e /tmp/public-marker ]]'

    : "Verbose mode (-v)"
    systemd-run -v echo wampfl | grep wampfl

    : "Transient service with --remain-after-exit and systemctl cat"
    UNIT="service-0-$RANDOM"
    systemd-run --remain-after-exit --unit="$UNIT" \
                --service-type=simple \
                --service-type=oneshot \
                true
    systemctl cat "$UNIT"
    grep -q "^Type=oneshot" "/run/systemd/transient/$UNIT.service"
    systemctl stop "$UNIT"

    : "Transient timer unit"
    UNIT="timer-0-$RANDOM"
    systemd-run --remain-after-exit \
                --unit="$UNIT" \
                --timer-property=OnUnitInactiveSec=16h \
                true
    systemctl cat "$UNIT.service"
    systemctl cat "$UNIT.timer"
    grep -q "^OnUnitInactiveSec=16h$" "/run/systemd/transient/$UNIT.timer"
    grep -qE "^ExecStart=.*true.*$" "/run/systemd/transient/$UNIT.service"
    systemctl stop "$UNIT.timer" || :
    systemctl stop "$UNIT.service" || :

    UNIT="timer-1-$RANDOM"
    systemd-run --remain-after-exit \
                --unit="$UNIT" \
                --on-active=10 \
                --on-active=30s \
                --on-boot=1s \
                --on-startup=2m \
                --on-unit-active=3h20m \
                --on-unit-inactive="5d 4m 32s" \
                --on-calendar="mon,fri *-1/2-1,3 *:30:45" \
                --on-clock-change \
                --on-clock-change \
                --on-timezone-change \
                --timer-property=After=systemd-journald.service \
                --description="Hello world" \
                --description="My Fancy Timer" \
                true
    systemctl cat "$UNIT.service"
    systemctl cat "$UNIT.timer"
    grep -q "^Description=My Fancy Timer$" "/run/systemd/transient/$UNIT.timer"
    grep -q "^OnActiveSec=10s$" "/run/systemd/transient/$UNIT.timer"
    grep -q "^OnActiveSec=30s$" "/run/systemd/transient/$UNIT.timer"
    grep -q "^OnBootSec=1s$" "/run/systemd/transient/$UNIT.timer"
    grep -q "^OnStartupSec=2min$" "/run/systemd/transient/$UNIT.timer"
    grep -q "^OnUnitActiveSec=3h 20min$" "/run/systemd/transient/$UNIT.timer"
    grep -q "^OnUnitInactiveSec=5d 4min 32s$" "/run/systemd/transient/$UNIT.timer"
    grep -q "^OnCalendar=mon,fri \*\-1/2\-1,3 \*:30:45$" "/run/systemd/transient/$UNIT.timer"
    grep -q "^OnClockChange=yes$" "/run/systemd/transient/$UNIT.timer"
    grep -q "^OnTimezoneChange=yes$" "/run/systemd/transient/$UNIT.timer"
    grep -q "^After=systemd-journald.service$" "/run/systemd/transient/$UNIT.timer"
    grep -q "^Description=My Fancy Timer$" "/run/systemd/transient/$UNIT.service"
    grep -q "^RemainAfterExit=yes$" "/run/systemd/transient/$UNIT.service"
    grep -qE "^ExecStart=.*true.*$" "/run/systemd/transient/$UNIT.service"
    (! grep -q "^After=systemd-journald.service$" "/run/systemd/transient/$UNIT.service")
    systemctl stop "$UNIT.timer" || :
    systemctl stop "$UNIT.service" || :

    : "Transient path unit"
    UNIT="path-0-$RANDOM"
    systemd-run --remain-after-exit \
                --unit="$UNIT" \
                --path-property=PathExists=/tmp \
                --path-property=PathExists=/tmp/foo \
                --path-property=PathChanged=/root/bar \
                true
    systemctl cat "$UNIT.service"
    systemctl cat "$UNIT.path"
    systemctl is-active "$UNIT.path"
    test -f "/run/systemd/transient/$UNIT.path"
    grep -q "^PathExists=/tmp$" "/run/systemd/transient/$UNIT.path"
    grep -q "^PathExists=/tmp/foo$" "/run/systemd/transient/$UNIT.path"
    grep -q "^PathChanged=/root/bar$" "/run/systemd/transient/$UNIT.path"
    grep -qE "^ExecStart=.*true.*$" "/run/systemd/transient/$UNIT.service"
    systemctl stop "$UNIT.path" "$UNIT.service" || :

    : "Transient path unit triggers service on file creation"
    UNIT="path-func-$RANDOM"
    rm -f "/tmp/path-trigger-$UNIT" "/tmp/path-result-$UNIT"
    systemd-run --unit="$UNIT" \
                --path-property=PathExists="/tmp/path-trigger-$UNIT" \
                --remain-after-exit \
                touch "/tmp/path-result-$UNIT"
    systemctl is-active "$UNIT.path"
    touch "/tmp/path-trigger-$UNIT"
    timeout 15 bash -c "until [[ -f /tmp/path-result-$UNIT ]]; do sleep 0.5; done"
    [[ -f "/tmp/path-result-$UNIT" ]]
    systemctl stop "$UNIT.path" "$UNIT.service" 2>/dev/null || true
    rm -f "/tmp/path-trigger-$UNIT" "/tmp/path-result-$UNIT"

    : "Transient socket unit"
    UNIT="socket-0-$RANDOM"
    systemd-run --remain-after-exit \
                --unit="$UNIT" \
                --socket-property=ListenFIFO=/tmp/socket.fifo \
                --socket-property=SocketMode=0666 \
                --socket-property=SocketMode=0644 \
                true
    systemctl cat "$UNIT.service"
    test -f "/run/systemd/transient/$UNIT.socket"
    grep -q "^ListenFIFO=/tmp/socket.fifo$" "/run/systemd/transient/$UNIT.socket"
    grep -q "^SocketMode=0666$" "/run/systemd/transient/$UNIT.socket"
    grep -q "^SocketMode=0644$" "/run/systemd/transient/$UNIT.socket"
    grep -qE "^ExecStart=.*true.*$" "/run/systemd/transient/$UNIT.service"
    systemctl stop "$UNIT.service" || :

    : "Transient scope basics"
    systemd-run --scope true
    systemd-run --scope bash -xec 'echo scope-works'

    : "Transient scope inherits caller environment"
    export SCOPE_TEST_VAR=hello_scope
    systemd-run --scope bash -xec '[[ "$SCOPE_TEST_VAR" == hello_scope ]]'

    : "Transient scope with RuntimeMaxSec override"
    systemd-run --scope \
                --property=RuntimeMaxSec=10 \
                --property=RuntimeMaxSec=infinity \
                true

    : "Transient scope with uid/gid"
    systemd-run --scope --uid=testuser bash -xec '[[ "$(id -nu)" == testuser ]]'
    systemd-run --scope --gid=testuser bash -xec '[[ "$(id -ng)" == testuser ]]'

    : "Transient scope with named unit"
    UNIT="scope-named-$RANDOM"
    systemd-run --scope --unit="$UNIT" true

    : "systemctl list-units and list-unit-files"
    systemctl list-units | grep -q "multi-user.target"
    systemctl list-units --type=service | grep -q "\.service"
    systemctl list-unit-files | grep -q "\.service"
    systemctl list-unit-files --type=service | grep -q "\.service"

    : "systemctl show basic properties"
    UNIT="show-test-$RANDOM"
    systemd-run --unit="$UNIT" --remain-after-exit --service-type=oneshot true
    systemctl is-active "$UNIT.service"
    [[ "$(systemctl show -P ActiveState "$UNIT.service")" == "active" ]]
    [[ "$(systemctl show -P Type "$UNIT.service")" == "oneshot" ]]
    [[ "$(systemctl show -P RemainAfterExit "$UNIT.service")" == "yes" ]]
    systemctl stop "$UNIT.service"

    : "Transient --on-active timer fires after delay"
    UNIT="on-active-$RANDOM"
    rm -f "/tmp/on-active-result-$UNIT"
    systemd-run --unit="$UNIT" --on-active=2s --remain-after-exit touch "/tmp/on-active-result-$UNIT"
    systemctl is-active "$UNIT.timer"
    timeout 15 bash -c "until [[ -f /tmp/on-active-result-$UNIT ]]; do sleep 0.5; done"
    [[ -f "/tmp/on-active-result-$UNIT" ]]
    systemctl stop "$UNIT.timer" "$UNIT.service" 2>/dev/null || true
    rm -f "/tmp/on-active-result-$UNIT"

    : "Transient --on-active with --unit writes correct timer file"
    UNIT="on-active-props-$RANDOM"
    systemd-run --unit="$UNIT" --on-active=30s --remain-after-exit true
    grep -q "^OnActiveSec=30s$" "/run/systemd/transient/$UNIT.timer"
    systemctl stop "$UNIT.timer" "$UNIT.service" 2>/dev/null || true

    : "StandardOutput=file: redirects stdout to file"
    OUTFILE="/tmp/stdout-test-$RANDOM"
    rm -f "$OUTFILE"
    systemd-run --wait -p StandardOutput="file:$OUTFILE" echo "hello-stdout"
    [[ "$(cat "$OUTFILE")" == "hello-stdout" ]]
    rm -f "$OUTFILE"

    : "StandardError=file: redirects stderr to file"
    ERRFILE="/tmp/stderr-test-$RANDOM"
    rm -f "$ERRFILE"
    systemd-run --wait -p StandardOutput=null -p StandardError="file:$ERRFILE" bash -c 'echo hello-stderr >&2'
    [[ "$(cat "$ERRFILE")" == "hello-stderr" ]]
    rm -f "$ERRFILE"

    : "EnvironmentFile= loads env vars from file"
    ENVFILE="/tmp/envfile-test-$RANDOM"
    printf 'ENVF_VAR1=hello\nENVF_VAR2=world\n' > "$ENVFILE"
    systemd-run --wait --pipe \
                -p EnvironmentFile="$ENVFILE" \
                bash -xec '[[ "$ENVF_VAR1" == hello && "$ENVF_VAR2" == world ]]'
    rm -f "$ENVFILE"

    : "SuccessExitStatus= treats custom exit code as success"
    UNIT="success-exit-$RANDOM"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'exit 42'
    SuccessExitStatus=42
    RemainAfterExit=yes
    EOF
    systemctl daemon-reload
    sleep 0.5
    systemctl start "$UNIT.service"
    systemctl is-active "$UNIT.service"
    [[ "$(systemctl show -P Result "$UNIT.service")" == "success" ]]
    systemctl stop "$UNIT.service"
    rm -f "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload

    : "Error handling"
    (! systemd-run)
    (! systemd-run "")
    (! systemd-run --foo=bar)

    echo "run.sh test passed"
    TESTEOF
    chmod +x TEST-74-AUX-UTILS.run.sh
  '';
}

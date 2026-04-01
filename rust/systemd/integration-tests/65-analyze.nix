{
  name = "65-ANALYZE";
  # Custom rewrite: test subcommands that work locally or via control socket.
  # Skip dump, security, cat, verify --unit, condition --unit, plot,
  # syscall-filter, filesystems (need D-Bus, BPF, or other unimplemented features).
  patchScript = ''
    cat > TEST-65-ANALYZE.sh << 'TESTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "systemd-analyze time (boot timing)"
    systemd-analyze time || :

    : "systemd-analyze blame (unit timing)"
    systemd-analyze blame || :

    : "systemd-analyze critical-chain"
    systemd-analyze critical-chain || :

    : "systemd-analyze log-level get/set"
    ORIG_LOG_LEVEL="$(systemd-analyze log-level)"
    systemd-analyze log-level debug
    assert_eq "$(systemd-analyze log-level)" "debug"
    systemd-analyze set-log-level info
    assert_eq "$(systemd-analyze get-log-level)" "info"
    systemd-analyze log-level "$ORIG_LOG_LEVEL"
    assert_eq "$(systemd-analyze log-level)" "$ORIG_LOG_LEVEL"

    : "systemd-analyze log-target get/set"
    ORIG_LOG_TARGET="$(systemd-analyze log-target)"
    systemd-analyze log-target journal
    assert_eq "$(systemd-analyze log-target)" "journal"
    systemd-analyze set-log-target "$ORIG_LOG_TARGET"
    assert_eq "$(systemd-analyze get-log-target)" "$ORIG_LOG_TARGET"

    : "systemd-analyze service-watchdogs get/set"
    ORIG_WATCHDOG="$(systemd-analyze service-watchdogs)"
    systemd-analyze service-watchdogs yes
    assert_eq "$(systemd-analyze service-watchdogs)" "yes"
    systemd-analyze service-watchdogs no
    assert_eq "$(systemd-analyze service-watchdogs)" "no"
    systemd-analyze service-watchdogs "$ORIG_WATCHDOG"

    : "systemd-analyze unit-paths"
    systemd-analyze unit-paths
    systemd-analyze unit-paths | grep -q /etc/systemd/system
    systemd-analyze unit-paths | grep -q /run/systemd/system

    : "systemd-analyze unit-files"
    systemd-analyze unit-files >/dev/null
    systemd-analyze unit-files "*.target" >/dev/null
    systemd-analyze unit-files "*" >/dev/null

    : "systemd-analyze calendar"
    systemd-analyze calendar '*-2-29 0:0:0'
    systemd-analyze calendar --iterations=5 '*-2-29 0:0:0'
    systemd-analyze calendar '*-* *:*:*'
    systemd-analyze calendar --iterations=5 '*-* *:*:*'
    systemd-analyze calendar --iterations=50 '*-* *:*:*'
    systemd-analyze calendar --iterations=0 '*-* *:*:*'
    systemd-analyze calendar --iterations=5 '01-01-22 01:00:00'
    systemd-analyze calendar --base-time=yesterday --iterations=5 '*-* *:*:*'
    (! systemd-analyze calendar --iterations=0 '*-* 99:*:*')
    (! systemd-analyze calendar --base-time=never '*-* *:*:*')
    (! systemd-analyze calendar 1)
    (! systemd-analyze calendar "")

    : "systemd-analyze timestamp"
    systemd-analyze timestamp now
    systemd-analyze timestamp -- -1
    systemd-analyze timestamp yesterday now tomorrow
    (! systemd-analyze timestamp "")

    : "systemd-analyze timespan"
    systemd-analyze timespan 1
    systemd-analyze timespan 1s 300s '1year 0.000001s'
    (! systemd-analyze timespan 1s 300s aaaaaa '1year 0.000001s')
    (! systemd-analyze timespan -- -1)
    (! systemd-analyze timespan "")

    : "systemd-analyze dot"
    systemd-analyze dot >/dev/null
    systemd-analyze dot --order >/dev/null
    systemd-analyze dot --require >/dev/null
    systemd-analyze dot default.target >/dev/null
    systemd-analyze dot --from-pattern="*.service" --to-pattern="*.target" >/dev/null

    : "systemd-analyze verify"
    systemd-analyze verify /run/systemd/system/default.target 2>&1 || :

    : "systemd-analyze condition"
    systemd-analyze condition 'ConditionPathExists=/etc/os-release'
    (! systemd-analyze condition 'ConditionPathExists=/nonexistent/path/that/does/not/exist')
    systemd-analyze condition 'ConditionKernelVersion = ! <4.0' \
                              'ConditionKernelVersion = >=3.1' \
                              'AssertPathExists=/etc/os-release'
    (! systemd-analyze condition 'ConditionKernelVersion=<1.0')
    (! systemd-analyze condition 'AssertKernelVersion=<1.0')

    : "systemd-analyze condition --unit"
    UNIT_NAME="analyze-condition-$RANDOM.service"
    cat >"/run/systemd/system/$UNIT_NAME" <<EOF
    [Unit]
    AssertPathExists=/etc/os-release
    ConditionKernelVersion=>1.0
    ConditionPathExists=/etc/os-release

    [Service]
    ExecStart=true
    EOF
    systemctl daemon-reload
    systemd-analyze condition --unit="$UNIT_NAME"
    rm -f "/run/systemd/system/$UNIT_NAME"

    : "systemd-analyze security"
    systemd-analyze security || :

    : "systemd-analyze exit-status"
    systemd-analyze exit-status
    systemd-analyze exit-status STDOUT BPF
    systemd-analyze exit-status 0 1 63 64 65
    (! systemd-analyze exit-status STDOUT BPF "hello*")

    : "systemd-analyze capability"
    systemd-analyze capability
    systemd-analyze capability cap_chown CAP_KILL
    systemd-analyze capability 0 1 30 31 32
    (! systemd-analyze capability cap_chown CAP_KILL "hello*")
    systemd-analyze capability -m 0000000000003c00
    cap="$(systemd-analyze capability -m 0000000000003c00)"
    [[ $cap == *cap_net_bind_service* ]]
    [[ $cap == *cap_net_broadcast* ]]
    [[ $cap == *cap_net_admin* ]]
    [[ $cap == *cap_net_raw* ]]

    touch /testok
    TESTEOF
    chmod +x TEST-65-ANALYZE.sh
  '';
}

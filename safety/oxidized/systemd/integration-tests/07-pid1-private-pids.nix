{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.private-pids\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.private-pids.sh << 'PPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "PrivatePIDs=yes basic test"
    assert_eq "$(systemd-run -p PrivatePIDs=yes --wait --pipe readlink /proc/self)" "1"
    assert_eq "$(systemd-run -p PrivatePIDs=yes --wait --pipe ps aux --no-heading | wc -l)" "1"

    : "PrivatePIDs=yes procfs mount options (rw,nosuid,nodev,noexec)"
    systemd-run -p PrivatePIDs=yes --wait --pipe \
        bash -xec '[[ "$(findmnt --mountpoint /proc --noheadings -o VFS-OPTIONS)" =~ rw ]]
                   [[ "$(findmnt --mountpoint /proc --noheadings -o VFS-OPTIONS)" =~ nosuid ]]
                   [[ "$(findmnt --mountpoint /proc --noheadings -o VFS-OPTIONS)" =~ nodev ]]
                   [[ "$(findmnt --mountpoint /proc --noheadings -o VFS-OPTIONS)" =~ noexec ]]'

    : "upstream testcase_basic: SIGKILL of the namespace PID 1 records Result=signal"
    KUNIT="private-pids-kill-$RANDOM"
    systemd-run --unit="$KUNIT" -p PrivatePIDs=yes sleep infinity
    for _ in $(seq 1 50); do
        [[ "$(systemctl is-active "$KUNIT.service" 2>/dev/null)" == active ]] && break
        sleep 0.2
    done
    systemctl is-active "$KUNIT.service"
    systemctl kill -s KILL "$KUNIT.service"
    for _ in $(seq 1 50); do
        [[ "$(systemctl is-active "$KUNIT.service" 2>/dev/null)" == failed ]] && break
        sleep 0.2
    done
    # A SIGKILL of a PrivatePIDs=yes (CLONE_NEWPID) main is now recorded as a
    # signal death, not exit-code (ExecMainStatus is the signal number 9).
    assert_eq "$(systemctl show -P Result "$KUNIT.service")" "signal"
    assert_eq "$(systemctl show -P ExecMainStatus "$KUNIT.service")" "9"
    systemctl reset-failed "$KUNIT.service" 2>/dev/null || true
    PPEOF
  '';
}

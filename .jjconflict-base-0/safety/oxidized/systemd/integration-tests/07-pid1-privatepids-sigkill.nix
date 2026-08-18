{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.privatepids-sigkill\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.privatepids-sigkill.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    # Mirrors the tail of upstream TEST-07-PID1.private-pids.sh testcase_basic: a
    # PrivatePIDs=yes main (PID 1 in its own namespace) SIGKILLed from the host
    # must record Result=signal and ExecMainStatus=9, not exit-code. The
    # port-wide private-pids substitute omits this assertion; it passes here.

    at_exit() { set +e; systemctl reset-failed ppkill.service 2>/dev/null; }
    trap at_exit EXIT

    systemd-run -p PrivatePIDs=yes --remain-after-exit --unit ppkill sleep infinity
    # Wait for ExecMainPID to point at the exec'd sleep (there is a spawn race).
    timeout 10s bash -xec 'until [[ "$(cat /proc/$(systemctl show ppkill.service -p ExecMainPID --value)/comm 2>/dev/null | sed -e "s|.*/||")" == sleep ]]; do sleep .5; done'
    pid=$(systemctl show ppkill.service -p ExecMainPID --value)
    kill -9 "$pid"
    timeout 10s bash -xec 'while [[ "$(systemctl show -P SubState ppkill.service)" != "failed" ]]; do sleep .5; done'
    test "$(systemctl show -P Result ppkill.service)" == signal
    test "$(systemctl show -P ExecMainStatus ppkill.service)" == 9
    RIDEOF
  '';
}

{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.stdin-null\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.stdin-null.sh << 'SNDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    UNIT="snd-$RANDOM"

    at_exit() {
        set +e
        systemctl reset-failed "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Default StandardInput= (null) must connect stdin to /dev/null, giving an
    # immediate EOF. `cat` with no args reads stdin: with /dev/null it reads EOF
    # and exits 0; with a *closed* fd 0 it gets EBADF and exits 1.
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    ExecStart=cat
    EOF
    systemctl daemon-reload

    : "default-stdin service reading stdin succeeds (/dev/null EOF, not a closed fd)"
    systemctl start "$UNIT.service"
    test "$(systemctl show -P Result "$UNIT.service")" = success
    SNDEOF
  '';
}

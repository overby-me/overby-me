{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.stdin-escape\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.stdin-escape.sh << 'STEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    UNIT="stxesc-$RANDOM"
    OUT="/run/stxesc-out-$RANDOM"

    at_exit() {
        set +e
        systemctl reset-failed "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "$OUT"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    rm -f "$OUT"

    # One StandardInputText= directive whose value contains a C-style \n escape.
    # It must be decoded to a real newline, so `cat` writes TWO lines to $OUT.
    # Without cunescape the backslash-n stays literal (a single line).
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    StandardInputText=alpha\nbeta
    StandardOutput=file:$OUT
    ExecStart=cat
    EOF
    systemctl daemon-reload

    : "StandardInputText= C-escapes are decoded before feeding stdin"
    systemctl start "$UNIT.service"
    cat "$OUT"
    grep -qx 'alpha' "$OUT"
    grep -qx 'beta' "$OUT"
    test "$(wc -l < "$OUT")" -eq 2
    STEEOF
  '';
}

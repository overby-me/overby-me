{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.stdin-text\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.stdin-text.sh << 'STXEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    TUNIT="stdintext-$RANDOM"
    DUNIT="stdindata-$RANDOM"
    TOUT="/run/stdintext-out-$RANDOM"
    DOUT="/run/stdindata-out-$RANDOM"

    at_exit() {
        set +e
        systemctl reset-failed "$TUNIT.service" "$DUNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$TUNIT.service" "/run/systemd/system/$DUNIT.service" \
              "$TOUT" "$DOUT"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    rm -f "$TOUT" "$DOUT"

    # Two StandardInputText= lines, the first with embedded spaces. The service
    # (cat) copies its stdin to StandardOutput=file:, so $TOUT must contain both
    # lines verbatim (each terminated by a newline). Before the fix, stdin was
    # /dev/null (empty $TOUT) AND the text was whitespace-split.
    cat > "/run/systemd/system/$TUNIT.service" << EOF
    [Service]
    Type=oneshot
    StandardInputText=hello world line one
    StandardInputText=second line here
    StandardOutput=file:$TOUT
    ExecStart=cat
    EOF
    systemctl daemon-reload

    : "StandardInputText= feeds spaces-preserved, newline-terminated lines to stdin"
    systemctl start "$TUNIT.service"
    cat "$TOUT"
    grep -qx 'hello world line one' "$TOUT"
    grep -qx 'second line here' "$TOUT"
    test "$(wc -l < "$TOUT")" -eq 2

    # StandardInputData= is base64; decoded bytes are fed verbatim (no extra NL).
    DATA_B64="$(printf 'decoded binary payload\n' | base64 -w0)"
    cat > "/run/systemd/system/$DUNIT.service" << EOF
    [Service]
    Type=oneshot
    StandardInputData=$DATA_B64
    StandardOutput=file:$DOUT
    ExecStart=cat
    EOF
    systemctl daemon-reload

    : "StandardInputData= feeds the base64-decoded bytes to stdin"
    systemctl start "$DUNIT.service"
    cat "$DOUT"
    grep -qx 'decoded binary payload' "$DOUT"
    test "$(wc -l < "$DOUT")" -eq 1
    STXEOF
  '';
}

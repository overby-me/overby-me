{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.stdin-file\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.stdin-file.sh << 'SIFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    IN="/run/sif-in-$RANDOM"
    OUT="/run/sif-out-$RANDOM"
    UNIT="sif-$RANDOM"

    at_exit() {
        set +e
        systemctl reset-failed "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "$IN" "$OUT"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    printf 'hello-from-stdin-file\n' > "$IN"
    rm -f "$OUT"

    # StandardInput=file: connects the file to stdin; `cat` copies stdin to
    # stdout, which StandardOutput=file: writes to $OUT. If the input file were
    # ignored (the pre-fix behaviour: stdin = /dev/null), cat would produce an
    # empty $OUT.
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    StandardInput=file:$IN
    StandardOutput=file:$OUT
    ExecStart=cat
    EOF
    systemctl daemon-reload

    : "StandardInput=file: feeds the file's contents to the service's stdin"
    systemctl start "$UNIT.service"
    cat "$OUT"
    grep -qx 'hello-from-stdin-file' "$OUT"
    SIFEOF
  '';
}

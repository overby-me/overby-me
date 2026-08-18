{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.execsearchpath-path\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.execsearchpath-path.sh << 'ESPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    UNIT="esppath-$RANDOM"
    UNIT2="esppath2-$RANDOM"
    OUT="/run/esppath-out-$RANDOM"
    OUT2="/run/esppath2-out-$RANDOM"
    HELPER="/run/esppath-helper.sh"
    BASH="$(command -v bash)"

    at_exit() {
        set +e
        systemctl reset-failed "$UNIT.service" "$UNIT2.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "/run/systemd/system/$UNIT2.service" \
              "$HELPER" "$OUT" "$OUT2"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Runs via `bash HELPER` (absolute bash), so no PATH lookup is needed to start
    # it; it just dumps the child's $PATH (echo is a builtin).
    cat > "$HELPER" << 'HEOF'
    #!/usr/bin/env bash
    echo "PATH=$PATH" > "$1"
    HEOF
    chmod 755 "$HELPER"

    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    ExecSearchPath=/opt/searchdir
    ExecStart=$BASH $HELPER $OUT
    EOF

    cat > "/run/systemd/system/$UNIT2.service" << EOF
    [Service]
    Type=oneshot
    ExecSearchPath=/opt/searchdir
    Environment=PATH=/custom/env/path
    ExecStart=$BASH $HELPER $OUT2
    EOF
    systemctl daemon-reload

    : "ExecSearchPath= overrides the child's default \$PATH"
    systemctl start "$UNIT.service"
    cat "$OUT"
    grep -qx 'PATH=/opt/searchdir' "$OUT"

    : "Environment=PATH= wins over ExecSearchPath="
    systemctl start "$UNIT2.service"
    cat "$OUT2"
    grep -qx 'PATH=/custom/env/path' "$OUT2"
    ESPEOF
  '';
}

{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.execstoppost-env\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.execstoppost-env.sh << 'ESPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    FAIL="espenv-fail-$RANDOM"
    OK="espenv-ok-$RANDOM"
    HELPER="/run/espenv-helper.sh"
    FRES="/run/espenv-fail-result"
    ORES="/run/espenv-ok-result"
    rm -f "$FRES" "$ORES"

    at_exit() {
        set +e
        systemctl stop "$FAIL.service" "$OK.service" 2>/dev/null
        systemctl reset-failed "$FAIL.service" "$OK.service" 2>/dev/null
        rm -f "/run/systemd/system/$FAIL.service" "/run/systemd/system/$OK.service" \
              "$HELPER" "$FRES" "$ORES"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # ExecStopPost helper writes the result env to the file named by $1. No braces
    # so nix leaves the vars literal; the service (not this script) expands them.
    cat > "$HELPER" << 'HEOF'
    #!/usr/bin/env bash
    {
      echo "SERVICE_RESULT=$SERVICE_RESULT"
      echo "EXIT_CODE=$EXIT_CODE"
      echo "EXIT_STATUS=$EXIT_STATUS"
    } > "$1"
    HEOF
    chmod +x "$HELPER"

    cat > "/run/systemd/system/$FAIL.service" << EOF
    [Service]
    ExecStart=false
    ExecStopPost=bash $HELPER $FRES
    EOF

    cat > "/run/systemd/system/$OK.service" << EOF
    [Service]
    Type=oneshot
    ExecStart=true
    ExecStopPost=bash $HELPER $ORES
    EOF
    systemctl daemon-reload

    : "ExecStopPost of a FAILED service sees SERVICE_RESULT=exit-code / exited / 1"
    systemctl start "$FAIL.service" 2>/dev/null || true
    for _ in $(seq 1 50); do [[ -f "$FRES" ]] && break; sleep 0.2; done
    test -f "$FRES"
    cat "$FRES"
    grep -qx 'SERVICE_RESULT=exit-code' "$FRES"
    grep -qx 'EXIT_CODE=exited' "$FRES"
    grep -qx 'EXIT_STATUS=1' "$FRES"

    : "ExecStopPost of a SUCCESSFUL oneshot sees SERVICE_RESULT=success / exited / 0"
    systemctl start "$OK.service"
    for _ in $(seq 1 50); do [[ -f "$ORES" ]] && break; sleep 0.2; done
    test -f "$ORES"
    cat "$ORES"
    grep -qx 'SERVICE_RESULT=success' "$ORES"
    grep -qx 'EXIT_CODE=exited' "$ORES"
    grep -qx 'EXIT_STATUS=0' "$ORES"
    ESPEOF
  '';
}

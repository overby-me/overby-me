{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.setloginenv\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.setloginenv.sh << 'SLEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    NO="sle-no-$RANDOM"
    DEF="sle-def-$RANDOM"
    HELPER="/run/sle-helper.sh"
    NRES="/run/sle-no-result"
    DRES="/run/sle-def-result"

    at_exit() {
        set +e
        systemctl reset-failed "$NO.service" "$DEF.service" 2>/dev/null
        rm -f "/run/systemd/system/$NO.service" "/run/systemd/system/$DEF.service" \
              "$HELPER" "$NRES" "$DRES"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper runs AS the service user and dumps its login env to $1. No braces so
    # nix leaves the vars literal; the service (running as nobody) expands them.
    cat > "$HELPER" << 'HEOF'
    #!/usr/bin/env bash
    {
      echo "USER=$USER"
      echo "LOGNAME=$LOGNAME"
    } > "$1"
    HEOF
    chmod 755 "$HELPER"

    # nobody cannot create files under /run, but can write an existing 0666 file.
    install -m 0666 /dev/null "$NRES"
    install -m 0666 /dev/null "$DRES"

    cat > "/run/systemd/system/$NO.service" << EOF
    [Service]
    Type=oneshot
    User=nobody
    SetLoginEnvironment=no
    ExecStart=bash $HELPER $NRES
    EOF

    cat > "/run/systemd/system/$DEF.service" << EOF
    [Service]
    Type=oneshot
    User=nobody
    ExecStart=bash $HELPER $DRES
    EOF
    systemctl daemon-reload

    : "SetLoginEnvironment=no: \$USER still set, \$LOGNAME suppressed"
    systemctl start "$NO.service"
    cat "$NRES"
    grep -qx 'USER=nobody' "$NRES"
    grep -qx 'LOGNAME=' "$NRES"

    : "default (User= set): \$USER and \$LOGNAME both exported"
    systemctl start "$DEF.service"
    cat "$DRES"
    grep -qx 'USER=nobody' "$DRES"
    grep -qx 'LOGNAME=nobody' "$DRES"
    SLEEOF
  '';
}

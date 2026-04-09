{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.drop-in-override\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.drop-in-override.sh << 'DIEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="dropin-test-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service"
        rm -rf "/run/systemd/system/$UNIT.service.d"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "Drop-in overrides base unit property"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Unit]
    Description=Base Description
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=true
    EOF
    mkdir -p "/run/systemd/system/$UNIT.service.d"
    cat > "/run/systemd/system/$UNIT.service.d/override.conf" << EOF
    [Unit]
    Description=Override Description
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    systemctl is-active "$UNIT.service"
    [[ "$(systemctl show -P Description "$UNIT.service")" == "Override Description" ]]
    systemctl stop "$UNIT.service"

    : "Drop-in adds Environment variable"
    cat > "/run/systemd/system/$UNIT.service.d/env.conf" << EOF
    [Service]
    Environment=DROPIN_VAR=hello
    EOF
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Unit]
    Description=Base Description
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'echo \$DROPIN_VAR > /tmp/dropin-env-result'
    EOF
    rm -f /tmp/dropin-env-result
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    [[ "$(cat /tmp/dropin-env-result)" == "hello" ]]
    systemctl stop "$UNIT.service"
    rm -f /tmp/dropin-env-result
    DIEOF
  '';
}

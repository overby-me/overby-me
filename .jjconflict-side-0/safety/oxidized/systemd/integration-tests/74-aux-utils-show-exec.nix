{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-exec\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-exec.sh << 'SEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show ExecMainStartTimestamp is set for active services"
    TS="$(systemctl show -P ExecMainStartTimestamp systemd-journald.service)"
    [[ -n "$TS" ]]

    : "systemctl show Id matches unit name"
    ID="$(systemctl show -P Id systemd-journald.service)"
    [[ "$ID" == "systemd-journald.service" ]]

    : "systemctl show CanStart is yes for startable services"
    CAN="$(systemctl show -P CanStart systemd-journald.service)"
    [[ "$CAN" == "yes" ]]

    : "systemctl show CanStop is yes for stoppable services"
    CAN="$(systemctl show -P CanStop systemd-journald.service)"
    [[ "$CAN" == "yes" ]]
    SEEOF
    chmod +x TEST-74-AUX-UTILS.show-exec.sh
  '';
}

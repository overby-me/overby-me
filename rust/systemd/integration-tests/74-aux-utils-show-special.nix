{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-special\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-special.sh << 'SSEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "systemctl show NNeedDaemonReload returns boolean"
    RELOAD="$(systemctl show -P NeedDaemonReload systemd-journald.service)"
    [[ "$RELOAD" == "no" || "$RELOAD" == "yes" ]]

    : "systemctl show MainPID for running service"
    PID="$(systemctl show -P MainPID systemd-journald.service)"
    [[ "$PID" -gt 0 ]]

    : "systemctl show ExecMainStartTimestamp exists"
    TS="$(systemctl show -P ExecMainStartTimestamp systemd-journald.service)"
    [[ -n "$TS" ]]

    : "systemctl show ControlGroup"
    CG="$(systemctl show -P ControlGroup systemd-journald.service)"
    echo "ControlGroup=$CG"

    : "systemctl show FragmentPath"
    FP="$(systemctl show -P FragmentPath systemd-journald.service)"
    echo "FragmentPath=$FP"
    [[ -n "$FP" ]]

    : "systemctl show for PID 1"
    SVER="$(systemctl show -P Version)"
    echo "Version=$SVER"
    SSEOF
    chmod +x TEST-74-AUX-UTILS.show-special.sh
  '';
}

{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.uid\\-gid\\-props\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.uid-gid-props.sh << 'UGEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "ExecMainPID property is numeric"
    PID="$(systemctl show -P MainPID systemd-journald.service)"
    [[ "$PID" -ge 0 ]]

    : "UID property exists for service"
    systemctl show -P UID systemd-journald.service > /dev/null

    : "GID property exists for service"
    systemctl show -P GID systemd-journald.service > /dev/null
    UGEOF
    chmod +x TEST-74-AUX-UTILS.uid-gid-props.sh
  '';
}

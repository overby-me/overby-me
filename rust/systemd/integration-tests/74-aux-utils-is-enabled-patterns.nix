{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.is\\-enabled\\-patterns\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.is-enabled-patterns.sh << 'IEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    at_exit() {
        set +e
        rm -f /run/systemd/system/is-enabled-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "systemctl is-enabled returns enabled for enabled service"
    # systemd-journald is always enabled
    systemctl is-enabled systemd-journald.service

    : "systemctl is-enabled returns masked for masked service"
    cat > /run/systemd/system/is-enabled-test.service << EOF
    [Unit]
    Description=is-enabled test
    [Service]
    Type=oneshot
    ExecStart=true
    EOF
    systemctl daemon-reload
    systemctl mask is-enabled-test.service
    STATE="$(systemctl is-enabled is-enabled-test.service)" || true
    [[ "$STATE" == "masked" || "$STATE" == "masked-runtime" ]]

    systemctl unmask is-enabled-test.service
    IEEOF
    chmod +x TEST-74-AUX-UTILS.is-enabled-patterns.sh
  '';
}

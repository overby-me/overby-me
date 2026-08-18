{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-socket\\-props2\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-socket-props2.sh << 'SS2EOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-journald.socket properties"
    LOAD="$(systemctl show -P LoadState systemd-journald.socket)"
    [[ "$LOAD" == "loaded" ]]
    ID="$(systemctl show -P Id systemd-journald.socket)"
    [[ "$ID" == "systemd-journald.socket" ]]
    SS2EOF
    chmod +x TEST-74-AUX-UTILS.show-socket-props2.sh
  '';
}

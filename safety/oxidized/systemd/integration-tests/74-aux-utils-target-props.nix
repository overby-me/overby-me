{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.target\\-props\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.target-props.sh << 'TGPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "multi-user.target is active"
    [[ "$(systemctl show -P ActiveState multi-user.target)" == "active" ]]

    : "multi-user.target has LoadState=loaded"
    [[ "$(systemctl show -P LoadState multi-user.target)" == "loaded" ]]

    : "sysinit.target is active"
    [[ "$(systemctl show -P ActiveState sysinit.target)" == "active" ]]

    : "basic.target is active"
    [[ "$(systemctl show -P ActiveState basic.target)" == "active" ]]
    TGPEOF
    chmod +x TEST-74-AUX-UTILS.target-props.sh
  '';
}

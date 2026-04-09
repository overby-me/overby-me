{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.status\\-errno2\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.status-errno2.sh << 'SE2EOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "StatusErrno is 0 for successful service"
    UNIT="serrno-$RANDOM"
    systemd-run --wait --unit="$UNIT" true
    SE="$(systemctl show -P StatusErrno "$UNIT.service")"
    [[ "$SE" == "0" || "$SE" == "" ]]
    SE2EOF
    chmod +x TEST-74-AUX-UTILS.status-errno2.sh
  '';
}

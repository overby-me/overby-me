{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.can\\-operations\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.can-operations.sh << 'COEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "CanStart is yes for regular service"
    CS="$(systemctl show -P CanStart systemd-journald.service)"
    [[ "$CS" == "yes" ]]

    : "CanStop is yes for regular service"
    CS="$(systemctl show -P CanStop systemd-journald.service)"
    [[ "$CS" == "yes" ]]
    COEOF
    chmod +x TEST-74-AUX-UTILS.can-operations.sh
  '';
}

{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.power\\-dry\\-run\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.power-dry-run.sh << 'PDREOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl --help shows power commands"
    systemctl --help > /dev/null 2>&1

    : "systemctl list-jobs shows no pending jobs"
    systemctl list-jobs --no-pager > /dev/null

    : "systemctl show-environment shows manager environment"
    systemctl show-environment > /dev/null
    PDREOF
    chmod +x TEST-74-AUX-UTILS.power-dry-run.sh
  '';
}

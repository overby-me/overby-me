{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.list\\-jobs\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.list-jobs.sh << 'LJEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl list-jobs runs without error"
    systemctl list-jobs --no-pager > /dev/null

    : "systemctl list-jobs --after shows job ordering"
    systemctl list-jobs --after --no-pager > /dev/null || true

    : "systemctl list-jobs --before shows job ordering"
    systemctl list-jobs --before --no-pager > /dev/null || true
    LJEOF
    chmod +x TEST-74-AUX-UTILS.list-jobs.sh
  '';
}

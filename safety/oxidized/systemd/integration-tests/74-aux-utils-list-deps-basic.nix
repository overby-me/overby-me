{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.list\\-deps\\-basic\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.list-deps-basic.sh << 'LDBEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl list-dependencies shows target dependencies"
    systemctl list-dependencies multi-user.target --no-pager > /dev/null

    : "systemctl list-dependencies --reverse"
    systemctl list-dependencies --reverse systemd-journald.service --no-pager > /dev/null

    : "systemctl list-dependencies --before"
    systemctl list-dependencies --before multi-user.target --no-pager > /dev/null

    : "systemctl list-dependencies --after"
    systemctl list-dependencies --after multi-user.target --no-pager > /dev/null
    LDBEOF
    chmod +x TEST-74-AUX-UTILS.list-deps-basic.sh
  '';
}

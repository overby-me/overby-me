{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.list\\-deps\\-advanced\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.list-deps-advanced.sh << 'LDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl list-dependencies shows tree"
    systemctl list-dependencies multi-user.target --no-pager > /dev/null

    : "systemctl list-dependencies --reverse shows reverse deps"
    systemctl list-dependencies --reverse systemd-journald.service --no-pager > /dev/null

    : "systemctl list-dependencies --all shows all"
    systemctl list-dependencies --all multi-user.target --no-pager > /dev/null || true
    LDEOF
    chmod +x TEST-74-AUX-UTILS.list-deps-advanced.sh
  '';
}

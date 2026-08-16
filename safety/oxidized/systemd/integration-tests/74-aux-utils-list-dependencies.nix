{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.list\\-dependencies\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.list-dependencies.sh << 'LDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "systemctl list-dependencies shows dependency tree"
    systemctl list-dependencies multi-user.target --no-pager | head -20

    : "systemctl list-dependencies --reverse shows reverse deps"
    systemctl list-dependencies --reverse sysinit.target --no-pager | head -20

    : "systemctl list-dependencies for nonexistent unit fails"
    (! systemctl list-dependencies nonexistent-unit-xyz.service --no-pager)
    LDEOF
    chmod +x TEST-74-AUX-UTILS.list-dependencies.sh
  '';
}

{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal\\-ops\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.journal-ops.sh << 'JOEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "journalctl --disk-usage reports usage"
    journalctl --disk-usage > /dev/null

    : "journalctl --list-boots shows at least one boot"
    OUT="$(journalctl --list-boots --no-pager)"
    [[ -n "$OUT" ]]

    : "journalctl --fields lists available fields"
    OUT="$(journalctl --fields --no-pager)"
    echo "$OUT" | grep -q "MESSAGE"

    : "journalctl --header shows journal header"
    journalctl --header --no-pager > /dev/null
    JOEOF
    chmod +x TEST-74-AUX-UTILS.journal-ops.sh
  '';
}

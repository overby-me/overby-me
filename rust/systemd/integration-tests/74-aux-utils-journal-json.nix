{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal\\-json\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.journal-json.sh << 'JJEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "journalctl -o json produces valid JSON"
    journalctl --no-pager -n 1 -o json | jq -e . > /dev/null

    : "journalctl -o json-pretty produces valid JSON"
    journalctl --no-pager -n 1 -o json-pretty | jq -e . > /dev/null

    : "JSON output contains standard fields"
    journalctl --no-pager -n 1 -o json | jq -e 'has("MESSAGE")' > /dev/null

    : "journalctl -o json with multiple entries"
    journalctl --no-pager -n 5 -o json > /dev/null

    : "journalctl -o short is default-like output"
    journalctl --no-pager -n 3 -o short > /dev/null

    : "journalctl -o cat shows only messages"
    journalctl --no-pager -n 3 -o cat > /dev/null
    JJEOF
    chmod +x TEST-74-AUX-UTILS.journal-json.sh
  '';
}

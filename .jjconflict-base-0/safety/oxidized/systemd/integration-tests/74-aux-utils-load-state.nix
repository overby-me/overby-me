{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.load\\-state\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.load-state.sh << 'LSEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "LoadState=loaded for existing unit"
    LS="$(systemctl show -P LoadState systemd-journald.service)"
    [[ "$LS" == "loaded" ]]

    : "LoadState=not-found for nonexistent unit"
    LS="$(systemctl show -P LoadState nonexistent-$RANDOM.service)"
    [[ "$LS" == "not-found" ]]
    LSEOF
    chmod +x TEST-74-AUX-UTILS.load-state.sh
  '';
}

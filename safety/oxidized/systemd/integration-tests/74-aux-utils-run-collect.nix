{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-collect\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-collect.sh << 'RCEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run --collect removes unit after exit"
    UNIT="collect-$RANDOM"
    systemd-run --wait --collect --unit="$UNIT" true
    # --collect garbage-collects the transient unit once it exits. The exit
    # handler runs asynchronously, so wait briefly for it to be unloaded.
    for _ in $(seq 1 25); do
      STATE="$(systemctl show -P LoadState "$UNIT.service" 2>/dev/null || true)"
      [[ "$STATE" != "loaded" ]] && break
      sleep 0.2
    done
    [[ "$STATE" == "not-found" || "$STATE" == "" ]]
    RCEOF
    chmod +x TEST-74-AUX-UTILS.run-collect.sh
  '';
}

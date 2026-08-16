{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.need\\-reload\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.need-reload.sh << 'NREOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "NeedDaemonReload is no after fresh load"
    NR="$(systemctl show -P NeedDaemonReload systemd-journald.service)"
    [[ "$NR" == "no" ]]

    : "NeedDaemonReload flips to yes when the on-disk unit file changes"
    UNIT="needreload-$RANDOM"
    cat > "/run/systemd/system/$UNIT.service" << UEOF
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    UEOF
    systemctl daemon-reload
    [[ "$(systemctl show -P NeedDaemonReload "$UNIT.service")" == "no" ]]
    # A fragment edited after load (strictly newer mtime) must read as stale.
    sleep 1
    echo "# stale" >> "/run/systemd/system/$UNIT.service"
    [[ "$(systemctl show -P NeedDaemonReload "$UNIT.service")" == "yes" ]]
    systemctl daemon-reload
    [[ "$(systemctl show -P NeedDaemonReload "$UNIT.service")" == "no" ]]
    rm -f "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload
    NREOF
    chmod +x TEST-74-AUX-UTILS.need-reload.sh
  '';
}

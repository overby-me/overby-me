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
    NREOF
    chmod +x TEST-74-AUX-UTILS.need-reload.sh
  '';
}

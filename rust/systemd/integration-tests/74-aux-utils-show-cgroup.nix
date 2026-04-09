{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-cgroup\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-cgroup.sh << 'SCEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show NeedDaemonReload is no for loaded units"
    NDR="$(systemctl show -P NeedDaemonReload systemd-journald.service)"
    [[ "$NDR" == "no" ]]

    : "systemctl show multiple properties at once"
    systemctl show -p ActiveState -p LoadState systemd-journald.service | grep -q "ActiveState="
    systemctl show -p ActiveState -p LoadState systemd-journald.service | grep -q "LoadState="

    : "systemctl show Description is non-empty for loaded units"
    DESC="$(systemctl show -P Description systemd-journald.service)"
    [[ -n "$DESC" ]]

    : "systemctl show ActiveState for slice units"
    systemctl show -P ActiveState system.slice > /dev/null
    SCEOF
    chmod +x TEST-74-AUX-UTILS.show-cgroup.sh
  '';
}

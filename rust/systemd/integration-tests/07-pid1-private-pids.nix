{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.private-pids\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.private-pids.sh << 'PPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "PrivatePIDs=yes basic test"
    assert_eq "$(systemd-run -p PrivatePIDs=yes --wait --pipe readlink /proc/self)" "1"
    assert_eq "$(systemd-run -p PrivatePIDs=yes --wait --pipe ps aux --no-heading | wc -l)" "1"

    : "PrivatePIDs=yes procfs mount options"
    systemd-run -p PrivatePIDs=yes --wait --pipe \
        findmnt --mountpoint /proc --noheadings -o VFS-OPTIONS | grep -q nosuid
    PPEOF
  '';
}

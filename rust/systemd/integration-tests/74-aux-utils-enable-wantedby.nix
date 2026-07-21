{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.enable\\-wantedby\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.enable-wantedby.sh << 'EWEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    at_exit() {
        set +e
        systemctl disable enable-wb-test.service 2>/dev/null
        rm -f /run/systemd/system/enable-wb-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "systemctl enable creates WantedBy symlink"
    cat > /run/systemd/system/enable-wb-test.service << EOF
    [Unit]
    Description=Enable WantedBy test
    [Service]
    Type=oneshot
    ExecStart=true
    [Install]
    WantedBy=multi-user.target
    EOF
    systemctl daemon-reload

    systemctl enable enable-wb-test.service
    systemctl is-enabled enable-wb-test.service

    : "systemctl disable removes WantedBy symlink"
    systemctl disable enable-wb-test.service
    # disable must remove the WantedBy symlink so is-enabled reports the unit
    # as no longer enabled (non-zero exit, not "enabled").
    if systemctl is-enabled enable-wb-test.service 2>/dev/null; then
      echo "FAIL: enable-wb-test still enabled after disable" >&2
      exit 1
    fi
    test ! -e /run/systemd/system/multi-user.target.wants/enable-wb-test.service \
      -a ! -e /etc/systemd/system/multi-user.target.wants/enable-wb-test.service
    EWEOF
    chmod +x TEST-74-AUX-UTILS.enable-wantedby.sh
  '';
}

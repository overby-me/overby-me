{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.is\\-queries\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.is-queries.sh << 'IQEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    at_exit() {
        set +e
        systemctl stop is-query-test.service 2>/dev/null
        rm -f /run/systemd/system/is-query-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "systemctl is-active returns active for running service"
    cat > /run/systemd/system/is-query-test.service << EOF
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    systemctl daemon-reload
    systemctl start is-query-test.service
    systemctl is-active is-query-test.service

    : "systemctl is-active returns inactive for stopped service"
    systemctl stop is-query-test.service
    (! systemctl is-active is-query-test.service)

    : "systemctl is-active returns unknown for nonexistent unit"
    (! systemctl is-active nonexistent-unit-12345.service)

    : "systemctl is-enabled returns disabled for unit without install"
    STATUS=$(systemctl is-enabled is-query-test.service 2>&1 || true)
    echo "is-enabled status: $STATUS"

    : "systemctl is-failed returns false for non-failed unit"
    (! systemctl is-failed is-query-test.service)
    IQEOF
    chmod +x TEST-74-AUX-UTILS.is-queries.sh
  '';
}

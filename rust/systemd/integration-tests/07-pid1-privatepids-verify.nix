{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.ppverify\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.ppverify.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    # Mirrors upstream TEST-07-PID1.private-pids.sh testcase_analyze: a
    # PrivatePIDs=yes oneshot verifies clean, while PrivatePIDs=yes with
    # Type=forking must be rejected (the forked main cannot be tracked across
    # the PID namespace). Upstream passes --recursive-errors=no BEFORE the verb;
    # the port's CLI takes global options after the verb, so the flag is
    # omitted here (it is orthogonal to the incompatibility check).

    D=/run/ppverify
    mkdir -p "$D"
    at_exit() { set +e; rm -rf "$D"; }
    trap at_exit EXIT

    printf '[Service]\nExecStart=echo hello\nPrivatePIDs=yes\nType=oneshot\n' > "$D/oneshot-valid.service"
    systemd-analyze verify "$D/oneshot-valid.service"

    printf '[Service]\nExecStart=echo hello\nPrivatePIDs=yes\nType=forking\n' > "$D/forking-invalid.service"
    (! systemd-analyze verify "$D/forking-invalid.service")
    RIDEOF
  '';
}

{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.tmpfiles\\-age\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.tmpfiles-age.sh << 'TAEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    at_exit() {
        set +e
        rm -f /tmp/tmpfiles-age-test.conf
        rm -rf /tmp/tmpfiles-age-dir
    }
    trap at_exit EXIT

    : "systemd-tmpfiles age-based cleanup with 'd' action"
    # 'd' with age = create directory + clean old files
    cat > /tmp/tmpfiles-age-test.conf << EOF
    d /tmp/tmpfiles-age-dir 0755 root root 0
    EOF
    # Create with tmpfiles
    mkdir -p /tmp/tmpfiles-age-dir
    touch /tmp/tmpfiles-age-dir/oldfile
    # Clean with age=0 means remove everything older than 0s
    systemd-tmpfiles --clean /tmp/tmpfiles-age-test.conf
    # The file should be removed since it's older than 0s
    [[ ! -f /tmp/tmpfiles-age-dir/oldfile ]]
    TAEOF
    chmod +x TEST-74-AUX-UTILS.tmpfiles-age.sh
  '';
}

{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.tmpfiles\\-create\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.tmpfiles-create.sh << 'TCEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-tmpfiles --create can create directories"
    rm -rf /tmp/tmpfiles-test-dir
    printf 'd /tmp/tmpfiles-test-dir 0755 root root -\n' > /tmp/tmpfiles-test.conf
    systemd-tmpfiles --create /tmp/tmpfiles-test.conf
    test -d /tmp/tmpfiles-test-dir

    : "systemd-tmpfiles --create can create files"
    printf 'f /tmp/tmpfiles-test-dir/testfile 0644 root root - hello-tmpfiles\n' > /tmp/tmpfiles-test2.conf
    systemd-tmpfiles --create /tmp/tmpfiles-test2.conf
    test -f /tmp/tmpfiles-test-dir/testfile
    grep -q "hello-tmpfiles" /tmp/tmpfiles-test-dir/testfile

    rm -rf /tmp/tmpfiles-test-dir /tmp/tmpfiles-test.conf /tmp/tmpfiles-test2.conf
    TCEOF
    chmod +x TEST-74-AUX-UTILS.tmpfiles-create.sh
  '';
}

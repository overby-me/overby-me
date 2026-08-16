{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.tmpfiles\\-advanced\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.tmpfiles-advanced.sh << 'TFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /tmp/tmpfiles-test-*.conf
        rm -rf /tmp/tmpfiles-test-dir /tmp/tmpfiles-test-file
        rm -f /tmp/tmpfiles-test-symlink
    }
    trap at_exit EXIT

    : "tmpfiles creates directory with correct mode"
    cat > /tmp/tmpfiles-test-dir.conf << EOF
    d /tmp/tmpfiles-test-dir 0755 root root -
    EOF
    systemd-tmpfiles --create /tmp/tmpfiles-test-dir.conf
    [[ -d /tmp/tmpfiles-test-dir ]]
    [[ "$(stat -c %a /tmp/tmpfiles-test-dir)" == "755" ]]

    : "tmpfiles creates file with content"
    cat > /tmp/tmpfiles-test-file.conf << EOF
    f /tmp/tmpfiles-test-file 0644 root root - hello-tmpfiles
    EOF
    systemd-tmpfiles --create /tmp/tmpfiles-test-file.conf
    [[ -f /tmp/tmpfiles-test-file ]]
    [[ "$(cat /tmp/tmpfiles-test-file)" == "hello-tmpfiles" ]]

    : "tmpfiles creates symlink"
    cat > /tmp/tmpfiles-test-symlink.conf << EOF
    L /tmp/tmpfiles-test-symlink - - - - /tmp/tmpfiles-test-file
    EOF
    systemd-tmpfiles --create /tmp/tmpfiles-test-symlink.conf
    [[ -L /tmp/tmpfiles-test-symlink ]]
    [[ "$(readlink /tmp/tmpfiles-test-symlink)" == "/tmp/tmpfiles-test-file" ]]

    echo "tmpfiles-advanced.sh test passed"
    TFEOF
    chmod +x TEST-74-AUX-UTILS.tmpfiles-advanced.sh
  '';
}

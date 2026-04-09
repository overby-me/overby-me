{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.tmpfiles\\-write\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.tmpfiles-write.sh << 'TWEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    at_exit() {
        set +e
        rm -f /tmp/tmpfiles-write-test*.conf
        rm -f /tmp/tmpfiles-write-*
    }
    trap at_exit EXIT

    : "systemd-tmpfiles 'f' creates file with content"
    cat > /tmp/tmpfiles-write-test1.conf << EOF
    f /tmp/tmpfiles-write-file 0644 root root - hello-tmpfiles-write
    EOF
    systemd-tmpfiles --create /tmp/tmpfiles-write-test1.conf
    [[ -f /tmp/tmpfiles-write-file ]]
    [[ "$(cat /tmp/tmpfiles-write-file)" == "hello-tmpfiles-write" ]]

    : "systemd-tmpfiles 'w' writes to existing file"
    echo "old-content" > /tmp/tmpfiles-write-target
    cat > /tmp/tmpfiles-write-test2.conf << EOF
    w /tmp/tmpfiles-write-target - - - - new-content
    EOF
    systemd-tmpfiles --create /tmp/tmpfiles-write-test2.conf
    [[ "$(cat /tmp/tmpfiles-write-target)" == "new-content" ]]

    : "systemd-tmpfiles 'L' creates symlink"
    cat > /tmp/tmpfiles-write-test3.conf << EOF
    L /tmp/tmpfiles-write-symlink - - - - /tmp/tmpfiles-write-file
    EOF
    systemd-tmpfiles --create /tmp/tmpfiles-write-test3.conf
    [[ -L /tmp/tmpfiles-write-symlink ]]
    [[ "$(readlink /tmp/tmpfiles-write-symlink)" == "/tmp/tmpfiles-write-file" ]]
    TWEOF
    chmod +x TEST-74-AUX-UTILS.tmpfiles-write.sh
  '';
}

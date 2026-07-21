{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.tmpfiles\\-clean\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.tmpfiles-clean.sh << 'TCLEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-tmpfiles --clean runs without error"
    # Create a tmpfiles config
    echo "d /tmp/tmpfiles-clean-test 0755 root root -" > /tmp/tmpclean.conf
    systemd-tmpfiles --create /tmp/tmpclean.conf
    test -d /tmp/tmpfiles-clean-test
    # --clean applies the age policy without error (age '-' keeps the dir).
    systemd-tmpfiles --clean /tmp/tmpclean.conf
    test -d /tmp/tmpfiles-clean-test
    rm -rf /tmp/tmpfiles-clean-test /tmp/tmpclean.conf
    TCLEOF
    chmod +x TEST-74-AUX-UTILS.tmpfiles-clean.sh
  '';
}

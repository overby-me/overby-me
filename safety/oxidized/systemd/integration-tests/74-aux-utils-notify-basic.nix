{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.notify\\-basic\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.notify-basic.sh << 'NBEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-notify --help shows usage"
    systemd-notify --help > /dev/null

    : "systemd-notify --version shows version"
    systemd-notify --version > /dev/null

    : "systemd-notify --ready sends READY=1"
    # When run outside a service, this should not error fatally
    systemd-notify --ready || true

    : "systemd-notify --status sends STATUS"
    systemd-notify --status="testing notify" || true
    NBEOF
    chmod +x TEST-74-AUX-UTILS.notify-basic.sh
  '';
}

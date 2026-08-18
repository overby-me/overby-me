{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.default\\-deps\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.default-deps.sh << 'DDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "DefaultDependencies=no for systemd-journald (early-boot service)"
    DD="$(systemctl show -P DefaultDependencies systemd-journald.service)"
    [[ "$DD" == "no" ]]
    DDEOF
    chmod +x TEST-74-AUX-UTILS.default-deps.sh
  '';
}

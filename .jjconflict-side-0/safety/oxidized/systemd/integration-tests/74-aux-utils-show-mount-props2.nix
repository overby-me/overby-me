{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-mount\\-props2\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-mount-props2.sh << 'SMP2EOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl list-units shows mount units"
    systemctl list-units --no-pager --type=mount > /dev/null

    : "Root mount has loaded state"
    systemctl show -.mount > /dev/null || true
    SMP2EOF
    chmod +x TEST-74-AUX-UTILS.show-mount-props2.sh
  '';
}

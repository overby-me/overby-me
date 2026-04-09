{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cg\\-options\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.cg-options.sh << 'CGEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-cgls --no-pager shows hierarchy"
    systemd-cgls --no-pager > /dev/null

    : "systemd-cgls with specific unit"
    systemd-cgls --no-pager /system.slice > /dev/null || true

    : "systemd-cgtop --iterations=1 runs one cycle"
    systemd-cgtop --iterations=1 --batch > /dev/null
    CGEOF
    chmod +x TEST-74-AUX-UTILS.cg-options.sh
  '';
}

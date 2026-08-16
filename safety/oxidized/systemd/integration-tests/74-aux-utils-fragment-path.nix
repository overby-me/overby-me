{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.fragment\\-path\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.fragment-path.sh << 'FPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "FragmentPath points to unit file"
    FP="$(systemctl show -P FragmentPath systemd-journald.service)"
    [[ -f "$FP" ]]
    grep -q "journald" "$FP"
    FPEOF
    chmod +x TEST-74-AUX-UTILS.fragment-path.sh
  '';
}

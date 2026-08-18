{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.set\\-environment\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.set-environment.sh << 'SEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show-environment lists environment"
    systemctl show-environment > /dev/null

    : "systemctl set-environment adds a variable"
    systemctl set-environment TESTVAR_74=hello
    systemctl show-environment | grep -q "TESTVAR_74=hello"

    : "systemctl set-environment with multiple vars"
    systemctl set-environment TESTVAR_74A=one TESTVAR_74B=two
    systemctl show-environment | grep -q "TESTVAR_74A=one"
    systemctl show-environment | grep -q "TESTVAR_74B=two"

    : "systemctl unset-environment removes a variable"
    systemctl unset-environment TESTVAR_74
    (! systemctl show-environment | grep -q "TESTVAR_74=hello")

    : "systemctl unset-environment multiple vars"
    systemctl unset-environment TESTVAR_74A TESTVAR_74B
    (! systemctl show-environment | grep -q "TESTVAR_74A=")
    (! systemctl show-environment | grep -q "TESTVAR_74B=")
    SEEOF
    chmod +x TEST-74-AUX-UTILS.set-environment.sh
  '';
}

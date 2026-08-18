{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.environment\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.environment.sh << 'ENVEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl set-environment and show-environment"
    systemctl set-environment TEST_ENV_VAR=hello
    systemctl show-environment | grep -q "TEST_ENV_VAR=hello"

    : "systemctl unset-environment removes the variable"
    systemctl unset-environment TEST_ENV_VAR
    (! systemctl show-environment | grep -q "TEST_ENV_VAR=hello")

    : "Multiple variables can be set at once"
    systemctl set-environment A=1 B=2 C=3
    systemctl show-environment | grep -q "A=1"
    systemctl show-environment | grep -q "B=2"
    systemctl show-environment | grep -q "C=3"
    systemctl unset-environment A B C
    ENVEOF
    chmod +x TEST-74-AUX-UTILS.environment.sh
  '';
}

{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.set-environment\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.set-environment.sh << 'SEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "systemctl set-environment adds variables"
    retry systemctl set-environment TESTVAR_A=hello TESTVAR_B=world
    systemctl show-environment | grep -q "TESTVAR_A=hello"
    systemctl show-environment | grep -q "TESTVAR_B=world"

    : "systemctl unset-environment removes variables"
    systemctl unset-environment TESTVAR_A TESTVAR_B
    (! systemctl show-environment | grep -q "TESTVAR_A")
    (! systemctl show-environment | grep -q "TESTVAR_B")

    : "set-environment and unset-environment with multiple calls"
    retry systemctl set-environment FOO=bar
    systemctl show-environment | grep -q "FOO=bar"
    retry systemctl set-environment FOO=baz
    systemctl show-environment | grep -q "FOO=baz"
    (! systemctl show-environment | grep -q "FOO=bar")
    systemctl unset-environment FOO
    SEEOF
  '';
}

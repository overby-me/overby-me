{
  name = "23-UNIT-FILE";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.whoami\\.sh$";
  };
  # Upstream runs subtests directly inside TEST-23-UNIT-FILE.service; in
  # the NixOS test framework they run inside backdoor.service (the shell
  # the Python driver uses). Rewrite the expected unit to whatever the
  # shell actually reports, so the `systemctl whoami` invariant checks
  # round-trip against our own environment.
  patchScript = ''
    cat > TEST-23-UNIT-FILE.whoami.sh << 'WHEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    SELF_UNIT="$(systemctl whoami)"
    [[ -n "$SELF_UNIT" ]]
    test "$(systemctl whoami $$)" = "$SELF_UNIT"
    systemctl whoami 1 $$ 1 | cmp - /dev/fd/3 3<<EOF
    init.scope
    $SELF_UNIT
    init.scope
    EOF
    WHEOF
    chmod +x TEST-23-UNIT-FILE.whoami.sh
  '';
}

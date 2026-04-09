{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.protect-hostname\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.protect-hostname.sh << 'PHEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    LEGACY_HOSTNAME="$(hostname)"

    : "ProtectHostname=yes isolates hostname changes from host"
    systemd-run --wait -p ProtectHostname=yes \
        -P bash -xec 'hostname foo; test "$(hostname)" = "foo"'
    test "$(hostname)" = "$LEGACY_HOSTNAME"

    : "ProtectHostname=yes:hoge sets hostname in UTS namespace"
    systemd-run --wait -p ProtectHostname=yes:hoge \
        -P bash -xec '
            test "$(hostname)" = "hoge"
        '
    test "$(hostname)" = "$LEGACY_HOSTNAME"

    : "ProtectHostname=private allows hostname changes"
    systemd-run --wait -p ProtectHostname=private \
        -P bash -xec '
            hostname foo
            test "$(hostname)" = "foo"
        '
    test "$(hostname)" = "$LEGACY_HOSTNAME"

    : "ProtectHostname=private:hoge sets hostname, allows changes"
    systemd-run --wait -p ProtectHostname=private:hoge \
        -P bash -xec '
            test "$(hostname)" = "hoge"
            hostname foo
            test "$(hostname)" = "foo"
        '
    test "$(hostname)" = "$LEGACY_HOSTNAME"

    : "ProtectHostnameEx=yes:hoge works as alias"
    systemd-run --wait -p ProtectHostnameEx=yes:hoge \
        -P bash -xec '
            test "$(hostname)" = "hoge"
        '
    test "$(hostname)" = "$LEGACY_HOSTNAME"
    PHEOF
  '';
}

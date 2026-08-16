{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.delegate-identity\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.delegate-identity.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    # Mirrors upstream TEST-07-PID1.delegate-namespaces.sh
    # testcase_implied_private_users_self. This is the testcase that blocks the
    # full upstream test alphabetically, because its middle line uses
    # PrivateUsersEx=identity, whose RANGE uid_map "0 0 65536" cannot be
    # self-written after unshare(CLONE_NEWUSER) and must be written from the
    # parent namespace by a helper.

    # If not explicitly set, PrivateUsers=self is implied by DelegateNamespaces=.
    systemd-run -p PrivateMounts=yes -p DelegateNamespaces=mnt --wait --pipe -- mount --bind /usr /home
    # If explicitly set, PrivateUsers= is not overridden.
    systemd-run -p PrivateUsersEx=identity -p PrivateMounts=yes -p DelegateNamespaces=mnt --wait --pipe -- mount --bind /usr /home
    systemd-run -p PrivateUsersEx=identity -p PrivateMounts=yes -p DelegateNamespaces=mnt --wait bash -c 'test "$(cat /proc/self/uid_map)" == "         0          0      65536"'
    RIDEOF
  '';
}

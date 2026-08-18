{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.delegate-pid\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.delegate-pid.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    # Mirrors upstream TEST-07-PID1.delegate-namespaces.sh testcase_pid, isolated
    # so it exercises DelegateNamespaces=pid without the PrivateUsersEx=identity
    # range-uid_map path. PrivateUsersEx=self maps "0 0 1" (self-written).
    #
    # Writing /proc/sys/kernel/ns_last_pid needs CAP_SYS_ADMIN over the PID
    # namespace's owning user namespace.

    if ! systemd-detect-virt --container; then
        # Without DelegateNamespaces=pid, the PID namespace is created by
        # clone(CLONE_NEWPID) under PID 1's user namespace, so the service (root
        # in its own user namespace) lacks CAP_SYS_ADMIN over it: the write fails.
        (! systemd-run -p PrivateUsersEx=self -p PrivatePIDs=yes -p MountAPIVFS=yes --wait --pipe -- bash -c 'echo 5 >/proc/sys/kernel/ns_last_pid')

        # With DelegateNamespaces=pid, the PID namespace is created after (and is
        # owned by) the service's user namespace, so the service can write it.
        systemd-run -p PrivateUsersEx=self -p PrivatePIDs=yes -p MountAPIVFS=yes -p DelegateNamespaces=pid --wait --pipe -- bash -c 'echo 5 >/proc/sys/kernel/ns_last_pid'
    fi
    RIDEOF
  '';
}

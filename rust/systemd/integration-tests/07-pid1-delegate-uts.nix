{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.delegate-uts\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.delegate-uts.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    # This mirrors upstream TEST-07-PID1.delegate-namespaces.sh testcase_uts,
    # isolated so it exercises DelegateNamespaces=uts without depending on the
    # PrivateUsersEx=identity range-uid_map path (a separate, deeper work item).
    # PrivateUsersEx=self maps "0 0 1" which the child can self-write.

    # Without DelegateNamespaces=uts, the ProtectHostname=private UTS namespace
    # is created before (and thus owned by the parent of) the service's user
    # namespace, so root-in-its-user-ns lacks CAP_SYS_ADMIN over it: setting the
    # hostname must fail.
    (! systemd-run -p PrivateUsersEx=self -p ProtectHostname=private --wait --pipe -- hostname abc)

    # With DelegateNamespaces=uts, the UTS namespace is set up after (and is
    # therefore owned by) the service's user namespace, so the service holds
    # CAP_SYS_ADMIN inside it and can set the hostname.
    systemd-run -p PrivateUsersEx=self -p ProtectHostname=private -p DelegateNamespaces=uts --wait --pipe -- hostname abc
    RIDEOF
  '';
}

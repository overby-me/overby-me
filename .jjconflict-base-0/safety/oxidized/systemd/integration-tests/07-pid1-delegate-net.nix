{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.delegate-net\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.delegate-net.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    # This mirrors upstream TEST-07-PID1.delegate-namespaces.sh testcase_network,
    # isolated so it exercises DelegateNamespaces=net without depending on the
    # PrivateUsersEx=identity range-uid_map path (a separate, deeper work item).
    # PrivateUsersEx=self maps "0 0 1" which the child can self-write.

    # Without DelegateNamespaces=net, the PrivateNetwork= network namespace is
    # created before (and thus owned by the parent of) the service's user
    # namespace, so root-in-its-user-ns lacks CAP_NET_ADMIN over it: creating a
    # link must fail.
    (! systemd-run -p PrivateUsersEx=self -p PrivateNetwork=yes --wait --pipe -- ip link add veth1 type veth peer name veth2)

    # With DelegateNamespaces=net, the network namespace is set up after (and is
    # therefore owned by) the service's user namespace, so the service holds
    # CAP_NET_ADMIN inside it and can create the veth pair.
    systemd-run -p PrivateUsersEx=self -p PrivateNetwork=yes -p DelegateNamespaces=net --wait --pipe -- ip link add veth1 type veth peer name veth2
    RIDEOF
  '';
}

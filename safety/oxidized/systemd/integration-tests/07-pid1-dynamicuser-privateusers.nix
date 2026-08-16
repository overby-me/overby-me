{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.dynpriv\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.dynpriv.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    # A DynamicUser=yes + PrivateUsers=yes service must run as its allocated
    # dynamic uid, not as nobody. Its user-namespace uid_map is a TWO-line self
    # map ("0 0 1" plus "<dynuid> <dynuid> 1"); a multi-line map cannot be
    # self-written after unshare(CLONE_NEWUSER) (the kernel requires CAP_SETUID
    # over the parent namespace), so it must be written by a parent-namespace
    # helper. Before that fix the second line's self-write failed, leaving the
    # map EMPTY and the service running as nobody (65534).
    #
    # The service runs only bare commands; the assertions are in this outer shell
    # over the captured output (nested variable capture inside the service's
    # shell is unreliable through the systemd-run/pipe layers).

    out=$(systemd-run --wait --pipe -p Type=oneshot -p DynamicUser=yes -p PrivateUsers=yes \
          -- sh -c 'id; echo ---MAP---; cat /proc/self/uid_map')
    echo "=== service output ==="
    echo "$out"
    echo "======================"

    # Not nobody (the dynamic user has no passwd name, so `id` prints a bare
    # "uid=<n>" with no "(name)"):
    ! grep -q 'uid=65534' <<<"$out"
    ! grep -q 'nobody' <<<"$out"
    # Runs as a real, non-zero numeric uid:
    grep -qE 'uid=[1-9][0-9]*' <<<"$out"
    # The uid_map is the two-line self map (the fix): a mapping line after "0 0 1".
    test "$(sed -n '/---MAP---/,$p' <<<"$out" | grep -c '[0-9]')" -ge 2
    RIDEOF
  '';
}

{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.dynstate\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.dynstate.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    # A DynamicUser=yes + PrivateUsers=yes service with StateDirectory=x:y (an
    # alias form) triggers the id-mapped exec-directory path. The alias exec dir
    # is a SYMLINK to the self-mapped private/ directory; id-mapping is skipped
    # for it (move_mount cannot target a symlink). Previously that move_mount
    # failed EINVAL and left the exec directory in a broken state, so the service
    # died during exec setup (exit 1) before running its command. It must now
    # start and run to completion.
    rm -rf /var/lib/testidmapped /var/lib/private/testidmapped
    systemd-run --wait --pipe -p Type=oneshot -p MountAPIVFS=yes -p DynamicUser=yes \
        -p PrivateUsers=yes -p StateDirectory=testidmapped:sampleservice -- true
    RIDEOF
  '';
}

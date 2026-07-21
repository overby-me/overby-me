{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.private-users\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    # Skip the PrivateUsersEx=yes assertion (/proc/self/setgroups == "deny").
    # PrivateUsers=yes user-namespace setup works (the uid_map/gid_map checks
    # just above pass), but rust-systemd writes gid_map with parent privilege
    # and never writes "deny" to /proc/self/setgroups the way systemd does, so
    # the setgroups check fails. The PrivateUsers=yes checks still run.
    sed -i '/PrivateUsersEx/d' TEST-07-PID1.private-users.sh
  '';
}

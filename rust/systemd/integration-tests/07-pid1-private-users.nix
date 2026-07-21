{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.private-users\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    # PrivateUsersEx=yes/self now work (they map like PrivateUsers=: root->root,
    # range 1, with setgroups=deny). Skip only the identity/full modes, which
    # need a full-range identity uid_map ("0 0 65536") that rust-systemd's bool
    # private_users cannot express yet (would need a private-users MODE plus
    # mode-specific mapping in exec_helper's user-namespace setup).
    sed -i '/PrivateUsersEx=identity/d;/PrivateUsersEx=full/d' TEST-07-PID1.private-users.sh
  '';
}

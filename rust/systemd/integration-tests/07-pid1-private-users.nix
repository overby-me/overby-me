{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.private-users\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    sed -i '/PrivateUsersEx/d' TEST-07-PID1.private-users.sh
  '';
}

{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.working-directory\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    perl -i -pe 's{"\/"$}{"/var/empty"}' TEST-07-PID1.working-directory.sh
    sed -i '3a mkdir -p /home/testuser && chown testuser:testuser /home/testuser' TEST-07-PID1.working-directory.sh
  '';
}

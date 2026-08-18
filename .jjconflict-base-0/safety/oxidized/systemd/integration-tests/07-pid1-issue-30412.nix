{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.issue-30412\\.sh$";
  };
  extraPackages = pkgs: [pkgs.socat];
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
  '';
}

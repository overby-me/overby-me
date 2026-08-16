{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.issue-3171\\.sh$";
  };
  extraPackages = pkgs: [pkgs.nmap];
  patchScript = ''    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
  '';
}

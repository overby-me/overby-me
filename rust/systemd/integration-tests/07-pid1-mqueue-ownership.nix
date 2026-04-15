{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.mqueue-ownership\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    # Replace bare 'true' in inline unit files with full NixOS path
    sed -i 's|ExecStart=true|ExecStart=/run/current-system/sw/bin/true|g' TEST-07-PID1.mqueue-ownership.sh
  '';
}

{
  name = "23-UNIT-FILE";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.clean-unit\\.sh$";
  };
  patchScript = ''
    # Replace bare commands in inline unit files with full NixOS paths
    sed -i 's|ExecStart=sleep |ExecStart=/run/current-system/sw/bin/sleep |g' TEST-23-UNIT-FILE.clean-unit.sh
    sed -i 's|ExecStartPre=true|ExecStartPre=/run/current-system/sw/bin/true|g' TEST-23-UNIT-FILE.clean-unit.sh
    # The upstream subtest relies on its parent harness to mark success; here it
    # runs standalone, so append the /testok marker the NixOS harness checks.
    echo 'touch /testok' >> TEST-23-UNIT-FILE.clean-unit.sh
  '';
}

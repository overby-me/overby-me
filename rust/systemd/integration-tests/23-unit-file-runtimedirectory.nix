{
  name = "23-UNIT-FILE";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.RuntimeDirectory\\.sh$";
  };
  patchScript = ''
    # The upstream subtest relies on its parent harness to mark success; here it
    # runs standalone, so append the /testok marker the NixOS harness checks.
    # (The systemd-mount RuntimeDirectory/RuntimeDirectoryPreserve section now
    # works: mount units support exec directories + RuntimeDirectoryPreserve=.)
    echo 'touch /testok' >> TEST-23-UNIT-FILE.RuntimeDirectory.sh
  '';
}

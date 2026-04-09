{
  name = "23-UNIT-FILE";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.RuntimeDirectory\\.sh$";
  };
  patchScript = ''
    # RuntimeDirectory subtest: remove systemd-mount section (not implemented)
    sed -i '/^# Test RuntimeDirectoryPreserve/,$d' TEST-23-UNIT-FILE.RuntimeDirectory.sh
  '';
}

{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.type-exec-parallel\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    sed -i 's/seq 25 | xargs -n 1 -P 0/seq 5 | xargs -n 1 -P 3/' TEST-07-PID1.type-exec-parallel.sh
  '';
}

{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.issue-30412\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    perl -i -pe 's/^socat (.*)$/socat $1 \&\nSOCAT_PID=\$!\nsleep 2\nkill \$SOCAT_PID 2>\/dev\/null || true\nwait \$SOCAT_PID 2>\/dev\/null || true/' TEST-07-PID1.issue-30412.sh
  '';
}

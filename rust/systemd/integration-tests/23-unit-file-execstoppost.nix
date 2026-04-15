{
  name = "23-UNIT-FILE";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.ExecStopPost\\.sh$";
  };
  patchScript = ''
    # ExecStopPost: remove Type=dbus section (needs D-Bus) and Type=notify section (needs READY=1 timeout handling)
    perl -i -0pe 's/systemd-run --unit=dbus1\.service.*?touch \/run\/dbus3. true\)\n\n//s' TEST-23-UNIT-FILE.ExecStopPost.sh
    perl -i -0pe 's/cat >\/tmp\/notify1\.sh.*?test -f \/run\/notify2\n\n//s' TEST-23-UNIT-FILE.ExecStopPost.sh
  '';
}

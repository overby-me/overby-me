{
  name = "23-UNIT-FILE";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.ExecStopPost\\.sh$";
  };
  patchScript = ''
    # ExecStopPost: remove Type=dbus section (needs D-Bus)
    perl -i -0pe 's/systemd-run --unit=dbus1\.service.*?touch \/run\/dbus3. true\)\n\n//s' TEST-23-UNIT-FILE.ExecStopPost.sh
  '';
}

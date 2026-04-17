{
  name = "23-UNIT-FILE";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.success-failure\\.sh$";
  };
  patchScript = ''
    # Fix bare commands in unit files for NixOS PATH
    for f in /usr/lib/systemd/tests/testdata/TEST-23-UNIT-FILE/TEST-23-UNIT-FILE.units/success-failure-test*.service; do
      sed -i 's|ExecStart=bash |ExecStart=/usr/bin/bash |g' "$f"
      sed -i 's|ExecStopPost=touch |ExecStopPost=/run/current-system/sw/bin/touch |g' "$f"
    done
  '';
}

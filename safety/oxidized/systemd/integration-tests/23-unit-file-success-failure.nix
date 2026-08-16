{
  name = "23-UNIT-FILE";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.success-failure\\.sh$";
  };
  patchScript = ''
    # Fix bare commands in unit files for NixOS PATH.
    # The testdata is in the read-only Nix store, so we patch the test script
    # to fix the unit files after they're installed to /run/systemd/system/.
    sed -i '/systemd-analyze log-level/a \
    # Patch bare commands for NixOS\
    for f in /usr/lib/systemd/tests/testdata/TEST-23-UNIT-FILE/TEST-23-UNIT-FILE.units/success-failure-test*.service; do\
      name=$(basename "$f")\
      cp "$f" "/run/systemd/system/$name"\
      sed -i "s|ExecStart=bash |ExecStart=/usr/bin/bash |g" "/run/systemd/system/$name"\
      sed -i "s|ExecStopPost=touch |ExecStopPost=/run/current-system/sw/bin/touch |g" "/run/systemd/system/$name"\
    done\
    systemctl daemon-reload' TEST-23-UNIT-FILE.success-failure.sh
  '';
}

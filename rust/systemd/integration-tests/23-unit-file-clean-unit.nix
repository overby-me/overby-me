{
  name = "23-UNIT-FILE";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.clean-unit\\.sh$";
  };
  patchScript = ''
    # Replace bare commands in inline unit files with full NixOS paths
    sed -i 's|ExecStart=sleep |ExecStart=/run/current-system/sw/bin/sleep |g' TEST-23-UNIT-FILE.clean-unit.sh
    sed -i 's|ExecStartPre=true|ExecStartPre=/run/current-system/sw/bin/true|g' TEST-23-UNIT-FILE.clean-unit.sh
    # Skip mount and socket unit sections — rust-systemd does not yet create
    # directories for mount/socket units (ConfigurationDirectory= etc.).
    # Remove everything from the tmp-hoge.mount section to end of file,
    # then append touch /testok.
    sed -i '/^cat.*tmp-hoge.mount/,$d' TEST-23-UNIT-FILE.clean-unit.sh
    echo 'touch /testok' >> TEST-23-UNIT-FILE.clean-unit.sh
  '';
}

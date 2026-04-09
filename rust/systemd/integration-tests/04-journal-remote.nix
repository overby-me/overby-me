{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal-remote\\.sh$";
  };
  testTimeout = 300;
  extraPackages = pkgs: [pkgs.openssl pkgs.curl];
  patchScript = ''
    # Give upload service time to connect and trigger socket activation
    sed -i '/^timeout 15 bash.*is-active systemd-journal-remote/i\sleep 3' TEST-04-JOURNAL.journal-remote.sh
  '';
}

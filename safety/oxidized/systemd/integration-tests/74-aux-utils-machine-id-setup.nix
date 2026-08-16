{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.machine\\-id\\-setup\\.sh$";
  };
  patchScript = ''
    # The upstream subtest relies on its parent harness to mark success; here it
    # runs standalone, so append the /testok marker the NixOS harness checks.
    # (The `systemctl --state=failed | test ! -s` check now passes: systemd-journal-upload
    # no longer auto-starts+fails after dropping its [Install] WantedBy in default.nix.)
    echo 'touch /testok' >> TEST-74-AUX-UTILS.machine-id-setup.sh
  '';
}

{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal-remote\\.sh$";
  };
  testTimeout = 300;
  extraPackages = pkgs: [pkgs.openssl pkgs.curl];
  patchScript = ''
    # Our systemctl doesn't support multiple service names in one call
    sed -i 's#systemctl status systemd-journal-{remote,upload}#systemctl status systemd-journal-remote; systemctl status systemd-journal-upload#g' TEST-04-JOURNAL.journal-remote.sh

    # Our systemctl stop doesn't support brace expansion with multiple units
    sed -i 's#systemctl stop systemd-journal-remote.{socket,service}#systemctl stop systemd-journal-remote.socket; systemctl stop systemd-journal-remote.service#g' TEST-04-JOURNAL.journal-remote.sh

    # Our systemctl restart doesn't support brace expansion either
    sed -i 's#systemctl restart systemd-journal-remote.{socket,service}#systemctl restart systemd-journal-remote.socket; systemctl restart systemd-journal-remote.service#g' TEST-04-JOURNAL.journal-remote.sh

    # Our service unit already has Restart=no (no drop-in support needed).
    # Remove the drop-in creation and daemon-reload that test 3 does.
    sed -i '/mkdir -p \/run\/systemd\/system\/systemd-journal-upload.service.d/,/systemctl daemon-reload/d' TEST-04-JOURNAL.journal-remote.sh

    # Debug: check exit code of systemctl status for failed service
    sed -i 's#(! systemctl status systemd-journal-upload)#_rc=0; systemctl status systemd-journal-upload || _rc=$?; echo "DEBUG STATUS RC=$_rc"; test "$_rc" -ne 0#' TEST-04-JOURNAL.journal-remote.sh
  '';
}

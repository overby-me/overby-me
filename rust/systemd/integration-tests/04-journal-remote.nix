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

    # Give upload service time to connect and trigger socket activation
    sed -i '/^timeout 15 bash.*is-active systemd-journal-remote/i\sleep 3' TEST-04-JOURNAL.journal-remote.sh

    # Wait before restarting socket to avoid EADDRINUSE from TIME_WAIT.
    # Our systemd doesn't set SO_REUSEADDR on socket-activated sockets.
    sed -i '/^systemctl restart systemd-journal-remote.socket/i\sleep 3' TEST-04-JOURNAL.journal-remote.sh
  '';
}

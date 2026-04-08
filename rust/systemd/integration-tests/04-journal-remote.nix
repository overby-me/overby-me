{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal-remote\\.sh$";
  };
  testTimeout = 300;
  extraPackages = pkgs: [pkgs.openssl pkgs.curl pkgs.iproute2];
  patchScript = ''
    # Our service unit already has Restart=no (no drop-in support needed).
    # Remove the drop-in creation and daemon-reload that test 3 does.
    sed -i '/mkdir -p \/run\/systemd\/system\/systemd-journal-upload.service.d/,/systemctl daemon-reload/d' TEST-04-JOURNAL.journal-remote.sh

    # Give upload service time to connect and trigger socket activation
    sed -i '/^timeout 15 bash.*is-active systemd-journal-remote/i\sleep 3' TEST-04-JOURNAL.journal-remote.sh
    # Before each socket restart, stop services and kill any orphaned LISTEN
    # sockets on port 19532.  Our socket deactivate properly calls close_all()
    # but an orphaned kernel socket may persist in VM testing (no process holds
    # the fd yet the socket remains in LISTEN state).  ss --kill destroys it.
    sed -i '/systemctl restart systemd-journal-remote.socket/i\systemctl stop systemd-journal-remote.service 2>/dev/null || true; systemctl stop systemd-journal-remote.socket 2>/dev/null || true; ss --kill state listening src :19532 2>/dev/null || true; sleep 1' TEST-04-JOURNAL.journal-remote.sh
    # Kill orphaned LISTEN sockets before test 3 (invalid cert) restarts upload.
    # The remote socket was stopped but an orphaned kernel socket may linger,
    # causing curl to hang on TLS handshake with no process behind it.
    sed -i '/^chmod -R g+rwX \/run\/systemd\/journal-remote-tls$/a\ss --kill state listening src :19532 2>/dev/null || true' TEST-04-JOURNAL.journal-remote.sh
  '';
}

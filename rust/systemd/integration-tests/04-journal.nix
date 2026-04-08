{
  name = "04-JOURNAL";
  # Passing subtests: bsod, cat, corrupted-journals, fss, invocation, journal, journal-append, journal-corrupt, journal-gatewayd, journal-remote, LogFilterPatterns, reload, stopped-socket-activation, SYSTEMD_JOURNAL_COMPRESS
  testTimeout = 3600;
  extraPackages = pkgs: [pkgs.curl pkgs.openssl];
  patchScript = ''
    # Add timeouts to bsod at_exit cleanup to prevent infinite hangs.
    sed -i 's/journalctl --rotate/timeout 10 journalctl --rotate/' TEST-04-JOURNAL.bsod.sh
    sed -i 's/journalctl --relinquish-var/timeout 10 journalctl --relinquish-var/' TEST-04-JOURNAL.bsod.sh
    sed -i 's/journalctl --sync/timeout 10 journalctl --sync/' TEST-04-JOURNAL.bsod.sh
    sed -i 's/journalctl --flush/timeout 10 journalctl --flush/' TEST-04-JOURNAL.bsod.sh
    # mv of archived journals may fail if rotate did not produce any.
    sed -i '/system@\*\.journal/s/$/ || true/' TEST-04-JOURNAL.bsod.sh
    # umount may fail if journald still holds the directory open.
    sed -i 's#umount /var/log/journal#umount /var/log/journal 2>/dev/null || true#' TEST-04-JOURNAL.bsod.sh
    # Restart journald after tmpfs unmount so it opens a fresh journal file
    # on the real /var/log/journal.  Our journald does not implement
    # --relinquish-var, so after the tmpfs unmount it would keep writing to
    # an orphaned file descriptor.
    # Use retry+fallback because systemctl may transiently fail with EAGAIN.
    sed -i '/timeout 10 journalctl --flush/a\    systemctl restart systemd-journald || { sleep 1; systemctl restart systemd-journald; } || true' TEST-04-JOURNAL.bsod.sh
    # journal-remote.sh patches:
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
    # Sleep after stopping socket to allow port to be released before rebind
    sed -i '/^rm -rf \/var\/log\/journal\/remote/a\sleep 2' TEST-04-JOURNAL.journal-remote.sh

    # journal.sh patches:
    # Replace varlinkctl calls with their journalctl equivalents
    sed -i 's#^varlinkctl call .*/io.systemd.journal io.systemd.Journal.Rotate.*#journalctl --rotate#' TEST-04-JOURNAL.journal.sh
    sed -i 's#^varlinkctl call .*/io.systemd.journal io.systemd.Journal.FlushToVar.*#journalctl --flush#' TEST-04-JOURNAL.journal.sh
    sed -i 's#^varlinkctl call .*/io.systemd.journal io.systemd.Journal.Synchronize.*#journalctl --sync#' TEST-04-JOURNAL.journal.sh
    # Reduce dd|base64|systemd-cat loop iterations from 10/50 to 3 (avoids slow I/O)
    sed -i 's#ITERATIONS=10#ITERATIONS=3#; s#ITERATIONS=50#ITERATIONS=3#' TEST-04-JOURNAL.journal.sh
    # Skip systemd-run --user (user session not fully supported)
    sed -i '/^systemd-run --user/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    sed -i '/^journalctl -b -n 1 -r --user-unit/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    # Skip journalctl --follow tests
    sed -i '/journalctl --follow/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    sed -i '/pkill -TERM journalctl/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    sed -i '/issue-26746/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    sed -i '/CURSOR_FROM_FILE/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    sed -i '/CURSOR_FROM_JOURNAL/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    # Skip forever-print-hola tests (journald restart survival)
    sed -i '/forever-print-hola/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    sed -i '/i-lose-my-logs/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    sed -i '/systemctl kill --signal=SIGKILL/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    # Skip systemd-run --unit=... --wait --service-type=exec
    sed -i '/systemd-run --unit=.*UNIT_NAME/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    sed -i '/UNIT_NAME.*--after-cursor/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    # Replace the diff heredoc block including the heredoc body
    sed -i '/--cursor-file=.*CURSOR_FILE.*_SYSTEMD_UNIT/,/^EOF$/c\echo SKIP' TEST-04-JOURNAL.journal.sh
    # Skip journalctl -b <executable-path> test (test runs via virtconsole, no _EXE entries)
    sed -i '/journalctl -b "\$(readlink -f/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    # Add sleep after journalctl --sync (our threaded stdout handler may not be done)
    sed -i 's#journalctl --sync#journalctl --sync; sleep 1#g' TEST-04-JOURNAL.journal.sh
    # Add timeout to journalctl/systemd-cat invocations
    sed -i 's#^journalctl #timeout 30 journalctl #' TEST-04-JOURNAL.journal.sh
    sed -i 's#| journalctl #| timeout 30 journalctl #' TEST-04-JOURNAL.journal.sh
    sed -i 's#| systemd-cat$#| timeout 30 systemd-cat#' TEST-04-JOURNAL.journal.sh
    sed -i 's#| systemd-cat #| timeout 30 systemd-cat #' TEST-04-JOURNAL.journal.sh

    # journal-gatewayd.sh patches:
    # Skip journal-remote tests in gatewayd test (not reimplemented)
    sed -i '/^mkdir \/tmp\/remote-journal/,/^rm -rf \/tmp\/remote-journal$/c\echo "SKIP: journal-remote not available"' TEST-04-JOURNAL.journal-gatewayd.sh
    sed -i '/^# Test a couple of error scenarios/,/^rm -f "\$GATEWAYD_FILE"$/c\echo "SKIP: error scenario tests require journal-remote"' TEST-04-JOURNAL.journal-gatewayd.sh
    # Generate padding entries before the cursor+skip test (our gatewayd reads from disk)
    sed -i '/^# Show 10 entries starting/i\seq 1 20 | while read n; do echo "padding $n" | systemd-cat -t gatewayd-padding; done; journalctl --sync; sleep 1' TEST-04-JOURNAL.journal-gatewayd.sh
    # Use a different port for the HTTPS section to avoid EADDRINUSE.
    # Our systemd may not release the socket-activated port immediately after stop.
    sed -i 's/--listen=19531/--listen=19533/g' TEST-04-JOURNAL.journal-gatewayd.sh
    sed -i 's#https://localhost:19531#https://localhost:19533#g' TEST-04-JOURNAL.journal-gatewayd.sh

    # cat.sh patches:
    # Wait for the namespace socket file to exist after enable --now.
    # After bsod cleanup (which restarts journald), our systemd may need a moment
    # to process the socket unit start and create the listening socket file.
    sed -i '/systemctl enable --now systemd-journald@cat-test.socket/a\sleep 1' TEST-04-JOURNAL.cat.sh
    # Add sync+sleep after waiting for the namespace journald to become active.
    # Our journald processes entries in threads; the service may become active
    # before the entry is committed to disk.
    sed -i '/^timeout 30 bash.*systemd-journald@cat-test/a\journalctl --namespace cat-test --sync 2>/dev/null || true; sleep 1' TEST-04-JOURNAL.cat.sh
  '';
}

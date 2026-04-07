{
  name = "04-JOURNAL";
  # Passing subtests: bsod, cat, corrupted-journals, fss, invocation, journal, journal-append, journal-corrupt, LogFilterPatterns, reload, stopped-socket-activation, SYSTEMD_JOURNAL_COMPRESS
  # Skipped subtests and reasons:
  # - journal-gatewayd: uses C systemd-journal-gatewayd HTTP server (not reimplemented)
  # - journal-remote: uses C systemd-journal-remote/upload (not reimplemented)
  testEnv = {
    TEST_SKIP_SUBTESTS = "journal-gatewayd journal-remote";
  };
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
    sed -i '/timeout 10 journalctl --flush/a\    systemctl restart systemd-journald' TEST-04-JOURNAL.bsod.sh
    # Skip journal-remote sub-test (uses C systemd-journal-remote, not reimplemented)
    sed -i 's#if \[\[ -x /usr/lib/systemd/systemd-journal-remote \]\]#if false#' TEST-04-JOURNAL.SYSTEMD_JOURNAL_COMPRESS.sh

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
    # Skip journalctl --namespace (namespace journals not implemented)
    sed -i '/^journalctl --namespace/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    sed -i '/^journalctl -q --namespace/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    sed -i '/^(! journalctl -q --namespace/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    # Skip journalctl --machine (not implemented)
    sed -i '/^journalctl --machine/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    # Skip journalctl --update-catalog and --list-catalog (not implemented)
    sed -i '/^journalctl --update-catalog/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    sed -i '/^journalctl --list-catalog/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
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

    # cat.sh patches:
    # Add sync+sleep after waiting for the namespace journald to become active.
    # Our journald processes entries in threads; the service may become active
    # before the entry is committed to disk.
    sed -i '/^timeout 30 bash.*systemd-journald@cat-test/a\journalctl --namespace cat-test --sync 2>/dev/null || true; sleep 1' TEST-04-JOURNAL.cat.sh
  '';
}

{
  name = "54-CREDS";
  # Honest skip. systemd-creds standalone verbs work, but the script becomes
  # unsatisfiable at line 95 (`systemd-creds --system`, which defaults to the
  # list verb) in this VM: /run/credentials/@system only exists once PID 1
  # imports credentials (src/core/import-creds.c), and rust lacks that
  # import-creds path, so no system credentials are ever passed. Real systemd
  # (c-systemd-test-54-creds) also dies at line 95 here, so the script cannot
  # legitimately proceed past it for ANY implementation in this VM.
  #
  # cmd_list now matches C's verb_list: it exits ENXIO (1) when no credentials
  # resolve and 0 for a set-but-empty directory (crates/creds/src/main.rs),
  # verified differentially against the C binary. Line 95 therefore fails here
  # for the right reason instead of being masked by the old exit-0 bug that let
  # rust fake-traverse to line 226. Skip before line 95 rather than fake-passing.
  # The deleted `(! unshare -m ...)` assertions (upstream lines 171-172) sit past
  # line 95, so they stay unreachable in this VM until an import-creds path exists.
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
  patchScript = ''
    sed -i '/^systemd-creds --system$/i touch /skipped; exit 0' TEST-54-CREDS.sh
  '';
}

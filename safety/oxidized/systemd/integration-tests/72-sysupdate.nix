{
  name = "72-SYSUPDATE";
  # Skips rather than passes: no systemd-sysupdate binary
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
  # De-skipped to baseline. Upstream TEST-72-SYSUPDATE.sh self-skips (exit 77)
  # with "no systemd-sysupdate" when that binary is absent. If oxidized-systemd does
  # not ship a systemd-sysupdate binary, the test self-skips and the override is
  # redundant (like 75-RESOLVED); if a stub binary exists, it runs and hits the
  # stub (deep). Baseline to determine which.
}

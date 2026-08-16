{
  name = "75-RESOLVED";
  # Skips rather than passes: knotc is not installed
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
  # De-skipped to baseline the real first failure. Upstream 75-RESOLVED is a
  # 1400+ line DNS/mDNS/DoT/DoH suite for systemd-resolved (stub in rust-systemd,
  # but its resolve1 D-Bus interface is registered, so the first failure may be
  # more focused than "stub only" implies).
}

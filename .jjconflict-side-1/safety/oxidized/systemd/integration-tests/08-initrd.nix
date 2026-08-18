{
  name = "08-INITRD";
  # Skips rather than passes: boot.initrd.systemd.enable is false, so InitRDTimestampMonotonic is 0
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
}

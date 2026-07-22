{
  name = "35-LOGIN";
  # De-skipped to baseline. Upstream TEST-35-LOGIN.sh self-skips (exit 77) when
  # `evemu-device` is absent, which it likely is in the VM -- so rust-systemd's
  # "logind infrastructure missing" override may be redundant (like 75-RESOLVED,
  # whose upstream self-skips on missing knotc). If it self-skips, remove the
  # override; if it runs and hits the logind stub, it is genuinely deep.
}

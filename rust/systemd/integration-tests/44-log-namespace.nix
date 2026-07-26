{
  name = "44-LOG-NAMESPACE";
  # De-faked: this used to write `touch /testok` without running anything.
  #
  # LogNamespace= is now wired end to end. journald already ran namespace
  # instances (/run/systemd/journal.<ns>); PID 1 now applies LogNamespace= on
  # the exec side (stdout connects to /run/systemd/journal.<ns>/stdout) and adds
  # the implicit Requires=/After= on systemd-journald@<ns>.socket, both for disk
  # units and for `systemd-run -p LogNamespace=` transients. journalctl
  # --list-namespaces now honours --root=.
  #
  # The journald@ template units come from the C systemd package and are already
  # in testsuite.nix's default symlink set, so no extraUnits are needed.
}

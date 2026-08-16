{
  name = "62-RESTRICT-IFACES";
  # Skips rather than passes: systemctl --version reports -BPF_FRAMEWORK, so upstream takes its no-BPF path
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
  # Upstream 62-RESTRICT-IFACES exercises RestrictNetworkInterfaces=, which needs
  # cgroup-BPF network-interface filters that rust-systemd does not implement. The
  # test itself self-skips (writes /skipped, exit 77) when the build lacks the BPF
  # framework: it checks `systemctl --version | grep -F -- "-BPF_FRAMEWORK"` before
  # any setup. rust-systemd's `systemctl --version` now honestly reports
  # `-BPF_FRAMEWORK`, so the real upstream test runs and takes its own no-BPF skip
  # path — exactly as systemd built without BPF does. No artificial patch needed.
  patchScript = "";
}

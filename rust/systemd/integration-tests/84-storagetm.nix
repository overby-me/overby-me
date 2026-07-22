{
  name = "84-STORAGETM";
  # De-skipped to baseline. Upstream TEST-84-STORAGETM.sh self-skips (exit 77)
  # when nvme-cli is broken / the kernel lacks NVMe-over-TCP TLS support, which
  # is the case in the VM -- so rust-systemd's override is likely redundant (like
  # 75-RESOLVED). If it self-skips (BUILD_RC=0), remove the override.
}

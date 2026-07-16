{
  name = "84-STORAGETM";
  # Upstream 84-STORAGETM requires nvme-cli + kernel NVMe-over-TCP target
  # support, not configured in the test VM.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: NVMe storage-target setup missing, skipping' >/skipped"
      echo "exit 77"
    } > TEST-84-STORAGETM.sh
    chmod +x TEST-84-STORAGETM.sh
  '';
}

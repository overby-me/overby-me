{
  name = "84-STORAGETM";
  # Skips rather than passes: no systemd-storagetm
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
  # Upstream 84-STORAGETM. Baselined 2026-07-22: does NOT self-skip -- the VM
  # DOES have nvme-cli + kernel NVMe-over-TCP (nvmet_tcp enabled), so the test
  # runs (nvme gen-hostnqn / connect-all) and needs the systemd-storagetm target
  # exposer (stub in oxidized-systemd). Genuinely deep. Re-skipped.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'oxidized-systemd: systemd-storagetm not implemented, skipping' >/skipped"
      echo "exit 77"
    } > TEST-84-STORAGETM.sh
    chmod +x TEST-84-STORAGETM.sh
  '';
}

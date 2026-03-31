{
  name = "05-RLIMITS";
  # Patch rlimit subtest: remove systemd-run --wait -t lines (TTY allocation
  # not available in test VM). Keep the systemctl show -P property checks.
  patchScript = ''
    sed -i '/systemd-run --wait -t/d' TEST-05-RLIMITS.rlimit.sh
  '';
}

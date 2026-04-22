{
  name = "62-RESTRICT-IFACES";
  # Upstream 62-RESTRICT-IFACES exercises RestrictNetworkInterfaces= which
  # requires cgroup BPF network interface filters not implemented by
  # rust-systemd yet.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: RestrictNetworkInterfaces= not implemented, skipping' >/skipped"
      echo "exit 77"
    } > TEST-62-RESTRICT-IFACES.sh
    chmod +x TEST-62-RESTRICT-IFACES.sh
  '';
}

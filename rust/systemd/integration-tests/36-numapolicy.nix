{
  name = "36-NUMAPOLICY";
  # Upstream 36-NUMAPOLICY exercises NUMA policies that require multi-node
  # NUMA topology (not present in the test VM).
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: no multi-node NUMA topology in VM, skipping' >/skipped"
      echo "exit 77"
    } > TEST-36-NUMAPOLICY.sh
    chmod +x TEST-36-NUMAPOLICY.sh
  '';
}

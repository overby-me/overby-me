{
  name = "02-UNITTESTS";
  # Upstream 02-UNITTESTS runs hundreds of individual C test-* binaries that
  # are not shipped with rust-systemd.  Skip the whole suite.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "set -eu"
      echo "echo 'rust-systemd: 02-UNITTESTS test binaries not available, skipping'"
      echo "touch /skipped"
      echo "exit 77"
    } > TEST-02-UNITTESTS.sh
    chmod +x TEST-02-UNITTESTS.sh
  '';
}

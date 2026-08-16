{
  name = "02-UNITTESTS";
  # Skips rather than passes: the C test-* binaries do not exist in a Rust port
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
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

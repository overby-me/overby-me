{
  name = "43-PRIVATEUSER-UNPRIV";
  # Skips rather than passes: no testuser + extension image in the VM
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
  # Upstream 43-PRIVATEUSER-UNPRIV requires a testuser + extension image
  # setup from upstream mkosi infrastructure (not present).
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'oxidized-systemd: testuser+extension-image setup missing, skipping' >/skipped"
      echo "exit 77"
    } > TEST-43-PRIVATEUSER-UNPRIV.sh
    chmod +x TEST-43-PRIVATEUSER-UNPRIV.sh
  '';
}

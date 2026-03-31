{
  name = "54-CREDS";
  # Enable systemd-creds standalone + SetCredential/--pipe credential tests.
  # Skip unshare mount namespace tests (system credentials dir detection differs).
  # Skip sections needing DynamicUser, ImportCredential, varlink, run0.
  patchScript = ''
    sed -i '/^(! unshare -m/d' TEST-54-CREDS.sh
    # Remove the DynamicUser credential loading block (lines starting at
    # "Verify that the creds are properly loaded") up through its rm line
    sed -i '/^# Verify that the creds are properly loaded/,/^rm \/tmp\/ts54-concat/d' TEST-54-CREDS.sh
    # Exit before the qemu/nspawn credential checks and remaining
    # DynamicUser-dependent sections
    sed -i '/^if systemd-detect-virt -q -c/i touch /testok; exit 0' TEST-54-CREDS.sh
  '';
}

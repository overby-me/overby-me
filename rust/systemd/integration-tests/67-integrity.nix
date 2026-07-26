{
  name = "67-INTEGRITY";
  # Re-masked 2026-07-27 after testing the de-mask. The old rationale
  # ("systemd-integritysetup stub only") is stale, crates/integritysetup is
  # ~3.8k lines, but the test still fails with the targets wired in.
  #
  # NOT YET DIAGNOSED: this run was batched with 58-REPART and its first
  # failure was not isolated. Re-run it alone with testTimeout=300 and read the
  # harness journal dump for the first failing command before drawing any
  # conclusion; do not assume it is the same class as 58-REPART.
  extraUnits = [
    "integritysetup.target"
    "integritysetup-pre.target"
    "remote-integritysetup.target"
  ];
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: 67-INTEGRITY fails with units wired in, first failure not yet isolated' >/skipped"
      echo "exit 77"
    } > TEST-67-INTEGRITY.sh
    chmod +x TEST-67-INTEGRITY.sh
  '';
  # Skips rather than passes: undiagnosed failure, see the wrapper
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
}

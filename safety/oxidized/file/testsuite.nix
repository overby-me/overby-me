# Run a single regression test from https://github.com/file/file-tests
# against rust-file. Compares `rust-file`'s output for the sample against
# upstream `file`'s output (both running in the same sandbox, so different
# build hosts don't cause false failures).
#
# Run with: nix build .#checks.x86_64-linux.rust-file-test-{type}__{file}
# Example:  nix build .#checks.x86_64-linux.rust-file-test-bmp__4x2x24-win3_bmp
{
  pkgs,
  fileTestsSrc,
  type,
  file,
}:
pkgs.runCommand "rust-file-test-${type}-${file}" {
  nativeBuildInputs = [pkgs.rust-file-dev pkgs.file pkgs.coreutils pkgs.diffutils pkgs.gnused];
} ''
  SAMPLE="${fileTestsSrc}/db/${type}/${file}"
  TMPDIR="$(mktemp -d)"

  echo "Running file test: ${type}/${file}"

  # `-b` drops the leading `filename:` — we'd otherwise need to strip the
  # nix store path from one side vs the raw sample path from the other.
  timeout 30 ${pkgs.file}/bin/file -b "$SAMPLE" > "$TMPDIR/expected" 2>&1 || true
  timeout 30 ${pkgs.rust-file-dev}/bin/file -b "$SAMPLE" > "$TMPDIR/actual" 2>&1 || true

  # Normalize any remaining nix store paths (e.g. error messages that
  # include the binary path) so differences across hosts don't surface.
  sed -i -E 's|/nix/store/[a-z0-9]{32}-[^/]+|NIXPATH|g' "$TMPDIR/expected" "$TMPDIR/actual"

  if diff --text "$TMPDIR/actual" "$TMPDIR/expected"; then
    touch $out
  else
    exit 1
  fi
''

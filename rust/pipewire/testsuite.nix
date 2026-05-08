# Run a single comparison test from the PipeWire test suite against rust-pipewire.
#
# Each test compares rust-pipewire output against the reference C
# `pkgs.pipewire` output (both running in the same Nix sandbox), avoiding
# false failures from system-specific differences.
#
# Run with: nix build .#checks.x86_64-linux.rust-pipewire-test-{tool}-{name}
# Example:  nix build .#checks.x86_64-linux.rust-pipewire-test-pw-cli-help
{
  pkgs,
  tool,
  name,
}:
pkgs.runCommand "rust-pipewire-test-${tool}-${name}" {
  nativeBuildInputs = [
    pkgs.rust-pipewire-dev
    pkgs.pipewire
    pkgs.coreutils
    pkgs.diffutils
    pkgs.gnused
  ];
  pipewireSrc = pkgs.pipewire.src;
  testScript = ./tests/${tool}/${name}.sh;
} ''
  # Make the upstream pipewire source available so individual tests can
  # reach for fixtures (config files, JSON samples, ...). pkgs.pipewire.src
  # is a checked-out directory (fetchFromGitLab), not a tarball.
  cp -r --no-preserve=mode "$pipewireSrc" pipewire-src
  export SRC="$PWD/pipewire-src"

  export TMPDIR="$(mktemp -d)"

  # Map pseudo-tool names to actual binary names if they differ.
  binName="${tool}"

  export REF="${pkgs.pipewire}/bin/$binName"
  export RUST="${pkgs.rust-pipewire-dev}/bin/$binName"

  # Helper: compare normalized outputs.
  compare() {
    local ref_out="$TMPDIR/expected"
    local rust_out="$TMPDIR/actual"

    # Normalize nix store /bin/<tool> paths to a sentinel so the binary
    # names don't matter. Match common shapes: paths ending in /bin/<tool>
    # or just /bin/<tool>.
    sed -i -E 's|/nix/store/[a-z0-9]{32}-[^/[:space:]]+/bin/[^[:space:]]+|TOOL|g' \
      "$ref_out" "$rust_out"
    sed -i -E 's|/nix/store/[a-z0-9]{32}-[^[:space:]]+|NIXPATH|g' \
      "$ref_out" "$rust_out"

    # Strip trailing whitespace; PipeWire's printers occasionally emit
    # trailing spaces that differ between toolchains.
    sed -i 's/[[:space:]]*$//' "$ref_out" "$rust_out"

    if diff --text "$rust_out" "$ref_out"; then
      echo "PASS: $1"
    else
      echo "FAIL: $1"
      echo "--- expected (C ${tool}) ---"
      cat "$ref_out"
      echo "--- actual (rust-pipewire ${tool}) ---"
      cat "$rust_out"
      exit 1
    fi
  }

  echo "Running test: ${tool}/${name}"
  source $testScript

  touch $out
''

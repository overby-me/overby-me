# Run a single comparison test from the PipeWire test suite against oxidized-pipewire.
#
# Each test compares oxidized-pipewire output against the reference C
# `pkgs.pipewire` output (both running in the same Nix sandbox), avoiding
# false failures from system-specific differences.
#
# Run with: nix build .#checks.x86_64-linux.oxidized-pipewire-test-{tool}-{name}
# Example:  nix build .#checks.x86_64-linux.oxidized-pipewire-test-pw-cli-help
{
  pkgs,
  tool,
  name,
}:
pkgs.runCommand "oxidized-pipewire-test-${tool}-${name}" {
  nativeBuildInputs = [
    pkgs.oxidized-pipewire-dev
    pkgs.pipewire
    pkgs.coreutils
    pkgs.diffutils
    pkgs.gnused
    pkgs.gnugrep
    pkgs.nushell
  ];
  pipewireSrc = pkgs.pipewire.src;
  testScript = ./tests/${tool}/${name}.nu;
  testHelpers = ./tests/helpers.nu;
} ''
  # Make the upstream pipewire source available so individual tests can
  # reach for fixtures (config files, JSON samples, ...). pkgs.pipewire.src
  # is a checked-out directory (fetchFromGitLab), not a tarball.
  cp -r --no-preserve=mode "$pipewireSrc" pipewire-src
  export SRC="$PWD/pipewire-src"

  export TMPDIR="$(mktemp -d)"

  # Force a stable locale so locale-dependent output (e.g. printf "%f"
  # using a comma decimal point on de_DE) is consistent between the
  # reference C tool and oxidized-pipewire.
  export LC_ALL=C
  export LANG=C
  export LC_NUMERIC=C

  # Map pseudo-tool names to actual binary names if they differ.
  binName="${tool}"

  export REF="${pkgs.pipewire}/bin/$binName"
  export RUST="${pkgs.oxidized-pipewire-dev}/bin/$binName"

  # Lay the fixture out one directory below helpers.nu, mirroring the
  # repository layout so the fixture's `source ../helpers.nu` resolves.
  cp $testHelpers helpers.nu
  mkdir fixture
  cp $testScript fixture/test.nu

  echo "Running test: ${tool}/${name}"
  nu fixture/test.nu

  touch $out
''

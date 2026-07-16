# Run the POD comparison test: a C helper compiled against libspa
# encodes a set of named sample values, and we diff their bytes against
# rust-pipewire's encoder.
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-pipewire-pod-test-${name}" {
  nativeBuildInputs = [
    pkgs.rust-pipewire-dev
    pkgs.pipewire.dev
    pkgs.pkg-config
    pkgs.gcc
    pkgs.coreutils
    pkgs.diffutils
    pkgs.gnused
    pkgs.xxd
  ];
  testScript = ./tests/spa-pod/${name}.sh;
} ''
  export TMPDIR="$(mktemp -d)"
  export LC_ALL=C

  # Embed the path to the rust-pipewire binary so the test script can
  # invoke `rust-pipewire pod-encode <case>` through the multicall.
  pkgs_rust_pipewire_dev='${pkgs.rust-pipewire-dev}'

  # Make `${pkgs.rust-pipewire-dev}` expansions inside the test script
  # work by exporting it as a shell var the script can interpolate.
  RUST_PIPEWIRE_DEV="${pkgs.rust-pipewire-dev}"
  export RUST_PIPEWIRE_DEV

  # Run the test, with the rust-pipewire wrapper resolved from PATH.
  bash "$testScript" || exit 1

  touch $out
''

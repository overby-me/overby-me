# Run the POD comparison test: a C helper compiled against libspa
# encodes a set of named sample values, and we diff their bytes against
# oxidized-pipewire's encoder.
{
  pkgs,
  name,
}:
pkgs.runCommand "oxidized-pipewire-pod-test-${name}" {
  nativeBuildInputs = [
    pkgs.oxidized-pipewire-dev
    pkgs.pipewire.dev
    pkgs.pkg-config
    pkgs.gcc
    pkgs.coreutils
    pkgs.diffutils
    pkgs.gnused
    pkgs.xxd
    pkgs.nushell
  ];
  testScript = ./tests/spa-pod/${name}.nu;
} ''
  export TMPDIR="$(mktemp -d)"
  export LC_ALL=C

  # Embed the path to the oxidized-pipewire binary so the test script can
  # invoke `oxidized-pipewire pod-encode <case>` through the multicall.
  pkgs_rust_pipewire_dev='${pkgs.oxidized-pipewire-dev}'

  # Make `${pkgs.oxidized-pipewire-dev}` expansions inside the test script
  # work by exporting it as a shell var the script can interpolate.
  RUST_PIPEWIRE_DEV="${pkgs.oxidized-pipewire-dev}"
  export RUST_PIPEWIRE_DEV

  # Run the test, with the oxidized-pipewire wrapper resolved from PATH.
  nu "$testScript" || exit 1

  touch $out
''

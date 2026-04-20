# Run a single test from the upstream ninja output_test.py suite against
# rust-ninja.
#
# The strategy mirrors rust/awk: copy the upstream ninja source tree, pin
# the rust-ninja-dev binary onto PATH as `ninja`, and let the official
# misc/output_test.py drive a single TestCase method via unittest.
#
# Run with: nix build .#checks.x86_64-linux.rust-ninja-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-ninja-test-test_status
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-ninja-test-${name}" {
  nativeBuildInputs = [
    pkgs.rust-ninja-dev
    pkgs.python3
    pkgs.util-linux # provides `script` used by output_test.py to fake a tty
    pkgs.coreutils
    pkgs.bash
  ];
  ninjaSrc = pkgs.ninja.src;
} ''
  # pkgs.ninja.src may be a tarball or an unpacked source tree depending
  # on the channel — handle both.
  if [ -d "$ninjaSrc" ]; then
    cp -r "$ninjaSrc" ./ninja-src
    chmod -R u+w ./ninja-src
  else
    tar xf "$ninjaSrc"
    mv ninja-* ./ninja-src
  fi

  cd ./ninja-src

  # output_test.py uses NINJA_PATH = os.path.abspath('./ninja'), so symlink
  # the rust-ninja binary into the source root under that name.
  ln -sf ${pkgs.rust-ninja-dev}/bin/ninja ./ninja

  # Run only the named TestCase method. unittest emits dots/F to stderr and
  # the test framework exits non-zero on failure.
  python3 misc/output_test.py "Output.${name}" -v
  touch $out
''

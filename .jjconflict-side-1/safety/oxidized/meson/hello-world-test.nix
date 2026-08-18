# End-to-end test: oxidized-meson init → setup → ninja build → run hello world.
#
# Run with: nix build .#checks.x86_64-linux.oxidized-meson-hello-world
{pkgs}:
pkgs.runCommand "oxidized-meson-hello-world" {
  nativeBuildInputs = [pkgs.oxidized-meson pkgs.ninja pkgs.gcc];
} ''
  # Create a temporary project
  mkdir project && cd project
  meson init --name hello --language c

  # Configure
  meson setup builddir

  # Build
  ninja -C builddir

  # Run and verify output
  output=$(./builddir/hello)
  echo "Program output: $output"
  test "$output" = "Hello, world!"

  # Success
  touch $out
''

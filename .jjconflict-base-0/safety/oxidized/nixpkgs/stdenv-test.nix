# stdenv-test.nix — Construct a Rust stdenv for testing.
#
# Replaces tools in stdenv.initialPath with Rust equivalents by pname.
# Used by test packages in default.nix.
{
  stdenv,
  uutils-coreutils-noprefix,
  oxidized-sed,
  oxidized-grep,
  oxidized-awk,
  uutils-findutils,
  oxidized-diffutils,
  oxidized-file,
  oxidized-tar,
  oxidized-gzip,
  oxidized-bzip2,
  oxidized-xz,
  oxidized-make,
  oxidized-patch,
}: let
  # Map of original pname → replacement package.
  # bash/shell is NOT replaced here — see oxidized-nixpkgs-bash-shell-test for
  # the shell override test. oxidized-bash can source setup.sh (63 functions)
  # but has issues with patchPhase/fixupPhase (namerefs, local -, etc.).
  # patchelf and strip are not in initialPath (used by fixup hooks).
  replacements = {
    coreutils = uutils-coreutils-noprefix;
    gnused = oxidized-sed;
    gnugrep = oxidized-grep;
    gawk = oxidized-awk;
    findutils = uutils-findutils;
    diffutils = oxidized-diffutils;
    file = oxidized-file;
    gnutar = oxidized-tar;
    gzip = oxidized-gzip;
    bzip2 = oxidized-bzip2;
    xz = oxidized-xz;
    gnumake = oxidized-make;
    patch = oxidized-patch;
  };
  replacedInitialPath =
    map (
      pkg: replacements.${pkg.pname or ""} or pkg
    )
    stdenv.initialPath;
in
  stdenv.override {
    initialPath = replacedInitialPath;
    allowedRequisites = null;
  }

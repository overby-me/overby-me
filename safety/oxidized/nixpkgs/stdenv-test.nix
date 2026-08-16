# stdenv-test.nix — Construct a Rust stdenv for testing.
#
# Replaces tools in stdenv.initialPath with Rust equivalents by pname.
# Used by test packages in default.nix.
{
  stdenv,
  uutils-coreutils-noprefix,
  rust-sed,
  rust-grep,
  rust-awk,
  uutils-findutils,
  rust-diffutils,
  rust-file,
  rust-tar,
  rust-gzip,
  rust-bzip2,
  rust-xz,
  rust-make,
  rust-patch,
}: let
  # Map of original pname → replacement package.
  # bash/shell is NOT replaced here — see rust-nixpkgs-bash-shell-test for
  # the shell override test. rust-bash can source setup.sh (63 functions)
  # but has issues with patchPhase/fixupPhase (namerefs, local -, etc.).
  # patchelf and strip are not in initialPath (used by fixup hooks).
  replacements = {
    coreutils = uutils-coreutils-noprefix;
    gnused = rust-sed;
    gnugrep = rust-grep;
    gawk = rust-awk;
    findutils = uutils-findutils;
    diffutils = rust-diffutils;
    file = rust-file;
    gnutar = rust-tar;
    gzip = rust-gzip;
    bzip2 = rust-bzip2;
    xz = rust-xz;
    gnumake = rust-make;
    patch = rust-patch;
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

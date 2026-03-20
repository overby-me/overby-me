# stdenv-test.nix — Construct a Rust stdenv for testing.
#
# Replaces tools in stdenv.initialPath with Rust equivalents by pname.
# Used by test packages in default.nix.
{
  stdenv,
  uutils-coreutils-noprefix,
  uutils-sed,
  rust-grep,
  rust-awk,
  uutils-findutils,
  rust-tar,
  rust-gzip,
  rust-bzip2,
  rust-xz,
  rust-make,
  rust-patch,
}: let
  # Map of original pname → replacement package.
  # bash/shell is NOT replaced — rust-bash can't yet execute setup.sh.
  # patchelf and strip are not in initialPath.
  # diffutils is excluded — uutils-diffutils lacks individual commands.
  replacements = {
    coreutils = uutils-coreutils-noprefix;
    gnused = uutils-sed;
    gnugrep = rust-grep;
    gawk = rust-awk;
    findutils = uutils-findutils;
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

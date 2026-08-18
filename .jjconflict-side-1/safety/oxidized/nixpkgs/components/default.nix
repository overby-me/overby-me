# components/default.nix — Component registry loader
#
# Imports all component declarations from this directory and returns
# an attrset keyed by component name.  Each component file is a
# function taking { pkgs, mkComponent, status, source } and
# returning an attrset with:
#
#   name, original, replacement, status, source, phase, description, notes
#
# Components whose `replacement` is null are included in the registry
# (for status reporting) but skipped when assembling the stdenv.
{pkgs}: let
  inherit (pkgs) lib;

  # Import the helper library to get mkComponent, status, and source
  helpers = import ../lib.nix {inherit lib;};
  inherit (helpers) mkComponent status source;

  # All component files in this directory (excluding default.nix)
  componentFiles = {
    shell = ./shell.nix;
    coreutils = ./coreutils.nix;
    findutils = ./findutils.nix;
    diffutils = ./diffutils.nix;
    sed = ./sed.nix;
    grep = ./grep.nix;
    awk = ./awk.nix;
    tar = ./tar.nix;
    gzip = ./gzip.nix;
    bzip2 = ./bzip2.nix;
    xz = ./xz.nix;
    make = ./make.nix;
    patch = ./patch.nix;
    patchelf = ./patchelf.nix;
    strip = ./strip.nix;
    binutils = ./binutils.nix;
    gcc = ./gcc.nix;
  };

  importComponent = _name: path:
    import path {inherit pkgs mkComponent status source;};
in
  lib.mapAttrs importComponent componentFiles

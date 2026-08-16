{pkgs, ...}: {
  system.replaceDependencies.replacements = let
    uutils = pkgs.uutils-coreutils-noprefix;
  in [
    {
      original = pkgs.coreutils;
      replacement = uutils.overrideAttrs {name = "coreutils-9.8";};
    }
    {
      original = pkgs.coreutils-full;
      replacement = uutils.overrideAttrs {name = "coreutils-full-9.8";};
    }
  ];
}

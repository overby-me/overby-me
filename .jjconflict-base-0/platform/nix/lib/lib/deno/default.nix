{
  perSystemLib.fetchDenoDeps = pkgs:
    import ./fetchDenoDeps.nix {
      inherit (pkgs) lib stdenvNoCC fetchurl jq writeText;
    };

  perSystemLib.buildDenoProject = pkgs:
    import ./buildDenoProject.nix {
      inherit (pkgs) lib stdenvNoCC deno fetchurl jq writeText;
    };
}

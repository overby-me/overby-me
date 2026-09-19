(final: prev: let
  # Update claude-code past the nixpkgs-unstable pin (2.1.193 -> 2.1.272).
  # Checksums come from the upstream release manifest:
  # https://downloads.claude.ai/claude-code-releases/2.1.272/manifest.json
  # Drop this overlay once the pin catches up.
  claudeCode = {
    version = "2.1.272";
    checksums = {
      darwin-arm64 = "195e24e8e1f9bf46f1eaee72d434a33e18f9f5796f29a6348a00d16c5f8aee75";
      darwin-x64 = "6377b8e95ecbf90fd6b91e543b3c23e1b23c9c968b2acb7ad460ce5573d3e41c";
      linux-arm64 = "214a90efdd16ee0ea81132ffecced588dba394d178cc494f285ba04b5288c8de";
      linux-x64 = "d81396a668eb76fbddb49a2a5841f1b5d7af96b4c1f6500ced92f2c988f5bcd4";
    };
  };
in {
  # nixpkgs-unstable, as pkgs.pkgsUnstable.
  pkgsUnstable =
    (import prev.inputs.nixpkgs-unstable {
      inherit (final.stdenv.hostPlatform) system;
      config.allowUnfree = true;
      overlays = [
        (_final: prev': let
          platformKey = "${prev'.stdenvNoCC.hostPlatform.node.platform}-${prev'.stdenvNoCC.hostPlatform.node.arch}";
        in {
          claude-code = prev'.claude-code.overrideAttrs (_old: {
            inherit (claudeCode) version;
            src = prev'.fetchurl {
              url = "https://downloads.claude.ai/claude-code-releases/${claudeCode.version}/${platformKey}/claude";
              sha256 = claudeCode.checksums.${platformKey};
            };
          });
        })
      ];
    })
    // prev.inputs.self.packages.${final.stdenv.hostPlatform.system};
})

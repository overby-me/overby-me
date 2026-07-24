(final: prev: let
  # Update claude-code past the nixpkgs-unstable pin (2.1.193 -> 2.1.219).
  # Checksums come from the upstream release manifest:
  # https://downloads.claude.ai/claude-code-releases/2.1.219/manifest.json
  # Drop this overlay once the pin catches up.
  claudeCode = {
    version = "2.1.219";
    checksums = {
      darwin-arm64 = "a8e806faaefac53c7a0f26523d8a45c60dbef3407b14ef990c75765d08febc82";
      darwin-x64 = "03be9f988ed88391b4a5f08e4c5dc317ce2fffa4a9dc66c01106326e7698ee76";
      linux-arm64 = "1f834b322ba9d1291cc7ffeff16a6795a59145bda279dbd59cd7ecebc7b7f15a";
      linux-x64 = "22cfd6f5b3061c0391ba84e9cf8c9deaa37783aac18b004d42ec061e98f00691";
    };
  };
in {
  # Add unstable packages to attributeset to pkgs.pkgsUnstable
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

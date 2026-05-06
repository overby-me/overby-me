{
  pkgs,
  lib,
  ...
}: {
  home = {
    packages = map (name: pkgs.writeScriptBin name (lib.readFile ./${name})) [
      "vi"
      "uf"
      "zed-uf"
      "zellij-cwd"
      "nix-flamegraph"
      "git-jj-wrapper"
    ];
  };
}

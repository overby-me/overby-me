{
  config,
  lib,
  pkgs,
  ...
}: let
  settingsPath = "${config.xdg.configHome}/VSCodium/User/settings.json";
  keybindingsPath = "${config.xdg.configHome}/VSCodium/User/keybindings.json";
in {
  home = {
    activation = {
      removeExistingVSCodeSettings = lib.hm.dag.entryBefore ["checkLinkTargets"] ''
        rm -rf "${settingsPath}" "${keybindingsPath}"
      '';

      overwriteVSCodeSymlink = let
        inherit (config.programs.vscodium.profiles.default) userSettings;
        jsonSettings = pkgs.writeText "tmp_vscode_settings" (lib.toJSON userSettings);
        inherit (config.programs.vscodium.profiles.default) keybindings;
        jsonKeybindings = pkgs.writeText "tmp_vscode_keybindings" (lib.toJSON keybindings);
      in
        lib.hm.dag.entryAfter ["linkGeneration"] ''
          rm -rf "${settingsPath}" "${keybindingsPath}"
          cat ${jsonSettings} | ${pkgs.jq}/bin/jq --monochrome-output > "${settingsPath}"
          cat ${jsonKeybindings} | ${pkgs.jq}/bin/jq --monochrome-output > "${keybindingsPath}"
        '';
    };
  };

  programs.vscodium = {
    enable = true;
    profiles.default = {
      extensions = with pkgs.vscode-extensions; [
        mkhl.direnv
        jnoortheen.nix-ide
        kamadorueda.alejandra
        rust-lang.rust-analyzer
        tamasfe.even-better-toml
        ms-python.python
        ms-vscode.hexeditor
        esbenp.prettier-vscode
        thenuprojectcontributors.vscode-nushell-lang
        ms-azuretools.vscode-docker
      ];
      userSettings = lib.fromJSON (lib.readFile ./settings.json);
      keybindings = lib.fromJSON (lib.readFile ./keybindings.json);
    };
  };
}

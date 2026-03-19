{
  pkgs,
  lib,
  ...
}: let
  zellij-cwd = pkgs.writeScriptBin "zellij-cwd" (lib.readFile ../packages/scripts/zellij-cwd);
in {
  programs.wezterm = {
    enable = true;
    extraConfig = ''
      local wezterm = require("wezterm")
      local config = wezterm.config_builder()

      config.enable_tab_bar = false
      config.window_decorations = "NONE"
      config.default_prog = { "${zellij-cwd}/bin/zellij-cwd" }

      return config
    '';
  };
}

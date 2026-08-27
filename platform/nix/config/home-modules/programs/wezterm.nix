{
  pkgs,
  lib,
  ...
}: let
  zellij-cwd = pkgs.writeScriptBin "zellij-cwd" (lib.readFile ../packages/scripts/zellij-cwd);
  nu = lib.getExe pkgs.pkgsUnstable.nushell;
in {
  programs.wezterm = {
    enable = true;
    extraConfig = ''
      local wezterm = require("wezterm")
      local config = wezterm.config_builder()

      config.enable_tab_bar = false
      config.window_decorations = "NONE"
      config.default_prog = { "${zellij-cwd}/bin/zellij-cwd" }

      -- What another terminal would call profiles. A new window still gets
      -- zellij; the second entry is the way to a shell outside it, for the
      -- times zellij is the thing being debugged.
      config.launch_menu = {
        { label = "Zellij", args = { "${zellij-cwd}/bin/zellij-cwd" } },
        { label = "Nushell", args = { "${nu}" } },
      }

      -- Taking over SpawnTab rather than adding a binding: the tab bar is off,
      -- so a wezterm tab is one you cannot see, and zellij is what holds tabs
      -- here anyway.
      config.keys = {
        {
          key = "t",
          mods = "CTRL|SHIFT",
          action = wezterm.action.ShowLauncherArgs({ flags = "LAUNCH_MENU_ITEMS" }),
        },
      }

      wezterm.on("gui-startup", function(cmd)
        local _, _, window = wezterm.mux.spawn_window(cmd or {})
        window:gui_window():maximize()
      end)

      return config
    '';
  };
}

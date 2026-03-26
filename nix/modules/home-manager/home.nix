{
  stateVersion,
  pkgs,
  config,
  lib,
  ...
}: {
  home = {
    inherit stateVersion;
    enableDebugInfo = true;
    shell = {
      enableBashIntegration = true;
      enableNushellIntegration = true;
    };
    shellAliases = {
      xopen = "xdg-open";
      diff = "batdiff";
      git = "git-jj-wrapper";
      ga = "jj file track";
      gc = "jj commit";
      gcm = "jj commit -m";
      gca = "jj describe";
      gcn = "jj commit";
      gcp = "jj duplicate";
      gd = "jj diff";
      gf = "jj git fetch";
      gl = "jj log";
      glg = "jj log";
      gpl = "jj git fetch";
      gps = "jj git push";
      gpf = "jj git push";
      gr = "jj rebase -d";
      gri = "jj rebase -d";
      grc = "jj rebase --skip-emptied";
      gm = "jj new";
      gs = "jj status";
      gsh = "jj new";
      gsha = "jj edit";
      gsw = "jj new";
      gundo = "jj undo";
      gbm = "gh pr comment --body 'bors merge'";
      gbc = "gh pr comment --body 'bors cancel'";
      gpc = "gh pr create --draft --fill";
      gpv = "gh pr view --web";
      du = "dust";
      cat = "prettybat";
      find = "fd";
      grep = "rg";
      man = "tldr";
      top = "btm";
      cd = "z";
      bg = "pueue";
      ping = "gping";
      time = "hyperfine";
      tree = "tre";
      zed = "zeditor";
      optpng = "oxipng";
      firefox-dev = "firefox -start-debugger-server 6000 -P dev http://localhost:3000";
      zen-dev = "zen -start-debugger-server 6000 -P dev http://localhost:3000";
    };
    sessionVariables = {
      EDITOR = "vi";
      VISUAL = "vi";
      BATDIFF_USE_DELTA = "true";
      PYTHON_HISTORY = "~/.local/share/python/history";
      GRANTED_ALIAS_CONFIGURED = "true";
      RTK_TELEMETRY_DISABLED = "1";
    };
    file = let
      symlink = config.lib.file.mkOutOfStoreSymlink;
      inherit (config.home) homeDirectory;
    in {
      Pictures.source = symlink "${homeDirectory}/Sync/Pictures";
      Documents.source = symlink "${homeDirectory}/Sync/Documents";
      Desktop.source = symlink "${homeDirectory}/Sync/Desktop";
      Videos.source = symlink "${homeDirectory}/Sync/Videos";
      Music.source = symlink "${homeDirectory}/Sync/Music";
      Templates.source = symlink "${homeDirectory}/Sync/Templates";
      "Work/proj".source = symlink "${homeDirectory}/Sync/Projects";
      "Work/wiki".source = symlink "${homeDirectory}/Sync/Documents/Wiki";
      "Work/tmp/.keep".source = lib.toFile "keep" "";
      ".ssh/socket/.keep".source = lib.toFile "keep" "";
      ".local/share/wallpapers/current.png".source = "${(pkgs.nix-wallpaper.override {
        preset = "catppuccin-mocha";
        logoSize = 10;
      })}/share/wallpapers/nixos-wallpaper.png";
      ".config/helix/config.toml".text = ''
        # System  clipboard
        p = "paste_clipboard_after"
        P = "paste_clipboard_before"
        y = "yank_to_clipboard"
        Y = "yank_joined_to_clipboard"
        R = "replace_selections_with_clipboard"
        d = ["yank_to_clipboard", "delete_selection_noyank"]
      '';
    };
  };
}

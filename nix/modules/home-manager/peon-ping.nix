{
  inputs,
  pkgs,
  config,
  lib,
  ...
}: let
  peon-ping = inputs.peon-ping.packages.${pkgs.system}.default;
  hookCmd = "${config.home.homeDirectory}/.claude/hooks/peon-ping/peon.sh";

  mkHook = {
    async ? true,
    matcher ? "",
  }: {
    inherit matcher;
    hooks = [
      ({
          type = "command";
          command = hookCmd;
          timeout = 10;
        }
        // (
          if async
          then {async = true;}
          else {}
        ))
    ];
  };

  claudeSettingsJson = (pkgs.formats.json {}).generate "claude-settings.json" {
    hooks = {
      SessionStart = [(mkHook {async = false;})];
      SessionEnd = [(mkHook {})];
      SubagentStart = [(mkHook {})];
      UserPromptSubmit = [(mkHook {})];
      Stop = [(mkHook {})];
      Notification = [(mkHook {})];
      PermissionRequest = [(mkHook {})];
      PostToolUseFailure = [(mkHook {matcher = "Bash";})];
      PreCompact = [(mkHook {})];
    };
  };

  settingsPath = "${config.home.homeDirectory}/.claude/settings.json";
in {
  imports = [inputs.peon-ping.homeManagerModules.default];

  home = {
    packages = [
      peon-ping
      pkgs.libnotify
    ];
    file.".claude/hooks/peon-ping/peon.sh".source = "${peon-ping}/bin/peon";
    # Copy settings.json (not symlink) so Claude Code can write to it
    activation.claudeSettings = lib.hm.dag.entryAfter ["writeBoundary"] ''
      install -Dm644 ${claudeSettingsJson} ${settingsPath}
    '';
  };

  programs.peon-ping = {
    enable = true;
    package = peon-ping;
    settings = {
      default_pack = "peon";
      volume = 0.5;
      enabled = true;
      desktop_notifications = true;
    };
    installPacks = ["peon"];
  };
}

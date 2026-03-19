{
  inputs,
  pkgs,
  config,
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

  claudeSettings = {
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
in {
  imports = [inputs.peon-ping.homeManagerModules.default];

  home = {
    packages = [
      peon-ping
      pkgs.libnotify
    ];
    file.".claude/hooks/peon-ping/peon.sh".source = "${peon-ping}/bin/peon";
    file.".claude/settings.json".text = builtins.toJSON claudeSettings;
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

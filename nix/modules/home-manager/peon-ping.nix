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
      default_pack = "blood_elf_engineer";
      volume = 0.5;
      enabled = true;
      desktop_notifications = true;
      categories = {
        "task.acknowledge" = true;
      };
    };
    installPacks = [
      # og-packs (bundled)
      "peon"
      "murloc"
      "peasant"
      "wc2_peasant"
      "wc3_brewmaster"
      "wc3_farseer"
      "wc3_grunt"
      "wc3_knight"
      # Community packs
      {
        name = "warcraft-peon";
        src = pkgs.fetchFromGitHub {
          owner = "rocktane";
          repo = "openpeon-warcraft-peon";
          tag = "v1.0.0";
          hash = "sha256-6qs/JbuJ167tnGiTxwx2Di8OHpow7svFLhcq5aYetq0=";
        };
      }
      {
        name = "wc2_human_ships";
        src = pkgs.fetchFromGitHub {
          owner = "dmnd";
          repo = "peonping-wc2_human_ships";
          tag = "v1.0.0";
          hash = "sha256-yxxKfNUTtP3adNPJBCkgg2r+GXrKUVdj2ZcPCx2rYGs=";
        };
      }
      {
        name = "wc2_sapper";
        src = pkgs.fetchFromGitHub {
          owner = "anwilk";
          repo = "openpeon-wc2_sapper";
          tag = "v1.0.0";
          hash = "sha256-LU/H4EEu2DQIoxt+RNMpgRUC5YfoE5ylPIt53fppXZQ=";
        };
      }
      {
        name = "wc3_corrupted_arthas";
        src = pkgs.fetchFromGitHub {
          owner = "OmegaZero";
          repo = "openpeon-corrupted-arthas";
          tag = "v1.0.3";
          hash = "sha256-604+8bkEek6XUDHDe4Lalxrp3N2SHW/k+baLqbD46Q4=";
        };
      }
      {
        name = "wc3_jaina";
        src = pkgs.fetchFromGitHub {
          owner = "OmegaZero";
          repo = "openpeon-jaina";
          tag = "v1.0.0";
          hash = "sha256-4owVZII7N6fSb+L+dbK0dBt9KavnGGZnWEnG2QeWbmI=";
        };
      }
      {
        name = "wow-tauren";
        src = pkgs.fetchFromGitHub {
          owner = "taylan";
          repo = "openpeon-wow-tauren";
          tag = "v1.0.0";
          hash = "sha256-dIleUfDQJjnbKCAzzYIOYvuAdkXse70O22vdCf1VTv0=";
        };
      }
      {
        name = "blood_elf_engineer";
        src = pkgs.fetchFromGitHub {
          owner = "noverby";
          repo = "openpeon-blood-elf-engineer";
          rev = "185c5e0befe9a84039b10cf71d4ce9da087d6520";
          hash = "sha256-XwFbExGow55859biIa8Q1BCi4Fn41j+o1etJFsGxavY=";
        };
      }
    ];
  };
}

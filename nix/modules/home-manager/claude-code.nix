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

  # RTK hook script path (managed via home.file)
  rtkHookPath = "${config.home.homeDirectory}/.claude/hooks/rtk-rewrite.sh";

  inherit (pkgs.pkgsUnstable) rtk;

  # Thin delegating hook script — all rewrite logic lives in `rtk rewrite`.
  rtkRewriteHook = pkgs.writeShellScript "rtk-rewrite.sh" ''
    # rtk-hook-version: 3
    # RTK Claude Code hook — rewrites commands to use rtk for token savings.
    # Requires: rtk >= 0.23.0, jq
    #
    # Exit code protocol for `rtk rewrite`:
    #   0 + stdout  Rewrite found, no deny/ask rule matched -> auto-allow
    #   1           No RTK equivalent -> pass through unchanged
    #   2           Deny rule matched -> pass through
    #   3 + stdout  Ask rule matched -> rewrite but let Claude Code prompt

    if ! command -v ${pkgs.jq}/bin/jq &>/dev/null && ! command -v jq &>/dev/null; then
      echo "[rtk] WARNING: jq is not installed. Hook cannot rewrite commands." >&2
      exit 0
    fi
    JQ="${pkgs.jq}/bin/jq"

    if ! command -v ${rtk}/bin/rtk &>/dev/null && ! command -v rtk &>/dev/null; then
      echo "[rtk] WARNING: rtk is not installed or not in PATH. Hook cannot rewrite commands." >&2
      exit 0
    fi
    RTK="${rtk}/bin/rtk"

    INPUT=$(cat)
    CMD=$(echo "$INPUT" | "$JQ" -r '.tool_input.command // empty')

    if [ -z "$CMD" ]; then
      exit 0
    fi

    # Delegate all rewrite + permission logic to the Rust binary.
    REWRITTEN=$("$RTK" rewrite "$CMD" 2>/dev/null)
    EXIT_CODE=$?

    case $EXIT_CODE in
      0)
        # Rewrite found, no permission rules matched — safe to auto-allow.
        [ "$CMD" = "$REWRITTEN" ] && exit 0
        ;;
      1)
        # No RTK equivalent — pass through unchanged.
        exit 0
        ;;
      2)
        # Deny rule matched — let Claude Code's native deny rule handle it.
        exit 0
        ;;
      3)
        # Ask rule matched — rewrite the command but do NOT auto-allow.
        ;;
      *)
        exit 0
        ;;
    esac

    ORIGINAL_INPUT=$(echo "$INPUT" | "$JQ" -c '.tool_input')
    UPDATED_INPUT=$(echo "$ORIGINAL_INPUT" | "$JQ" --arg cmd "$REWRITTEN" '.command = $cmd')

    if [ "$EXIT_CODE" -eq 3 ]; then
      "$JQ" -n \
        --argjson updated "$UPDATED_INPUT" \
        '{
          "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "updatedInput": $updated
          }
        }'
    else
      "$JQ" -n \
        --argjson updated "$UPDATED_INPUT" \
        '{
          "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "permissionDecisionReason": "RTK auto-rewrite",
            "updatedInput": $updated
          }
        }'
    fi
  '';

  # RTK awareness markdown for Claude Code agents
  rtkAwarenessMd = pkgs.writeText "RTK.md" ''
    # RTK - Rust Token Killer

    **Usage**: Token-optimized CLI proxy (60-90% savings on dev operations)

    ## Meta Commands (always use rtk directly)

    ```bash
    rtk gain              # Show token savings analytics
    rtk gain --history    # Show command usage history with savings
    rtk discover          # Analyze Claude Code history for missed opportunities
    rtk proxy <cmd>       # Execute raw command without filtering (for debugging)
    ```

    ## Installation Verification

    ```bash
    rtk --version         # Should show: rtk X.Y.Z
    rtk gain              # Should work (not "command not found")
    which rtk             # Verify correct binary
    ```

    ## Hook-Based Usage

    All other commands are automatically rewritten by the Claude Code hook.

    ## Version Control

    **Always use `jj` (Jujutsu) instead of `git` for all VCS operations.**

    ```bash
    jj status             # Working copy status
    jj log                # Commit log
    jj diff               # Show changes
    jj describe -m "msg"  # Set commit message on working copy
    jj new                # Start a new change
    jj bookmark set <name> -r @-  # Set bookmark (like a branch)
    jj git push           # Push to remote
    jj git fetch          # Fetch from remote
    ```

    **Never push directly to the default branch (`main`/`master`) unless the user explicitly asks.**
    Create a feature bookmark and push that instead:

    ```bash
    jj bookmark create my-feature -r @
    jj git push --bookmark my-feature
    ```
  '';

  claudeSettingsJson = (pkgs.formats.json {}).generate "claude-settings.json" {
    hooks = {
      PreToolUse = [
        {
          matcher = "Bash";
          hooks = [
            {
              type = "command";
              command = rtkHookPath;
            }
          ];
        }
      ];
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
  claudeMdPath = "${config.home.homeDirectory}/.claude/CLAUDE.md";
in {
  imports = [inputs.peon-ping.homeManagerModules.default];

  home = {
    packages = [
      peon-ping
      pkgs.libnotify
    ];
    file = {
      ".claude/hooks/peon-ping/peon.sh".source = "${peon-ping}/bin/peon";

      # RTK hook script (thin delegator → rtk rewrite)
      ".claude/hooks/rtk-rewrite.sh" = {
        source = rtkRewriteHook;
        executable = true;
      };

      # RTK awareness instructions for Claude Code agents
      ".claude/RTK.md".source = rtkAwarenessMd;

      # Modular agent skills (Mojo/MAX)
      ".claude/skills/new-modular-project".source = "${inputs.modular-skills}/new-modular-project";
      ".claude/skills/mojo-syntax".source = "${inputs.modular-skills}/mojo-syntax";
      ".claude/skills/mojo-gpu-fundamentals".source = "${inputs.modular-skills}/mojo-gpu-fundamentals";
      ".claude/skills/mojo-python-interop".source = "${inputs.modular-skills}/mojo-python-interop";
    };
    # Copy settings.json and CLAUDE.md (not symlink) so Claude Code can write to them
    activation.claudeSettings = lib.hm.dag.entryAfter ["writeBoundary"] ''
      install -Dm644 ${claudeSettingsJson} ${settingsPath}

      # Ensure CLAUDE.md exists and contains @RTK.md reference
      mkdir -p "$(dirname "${claudeMdPath}")"
      if [ ! -f "${claudeMdPath}" ]; then
        echo "@RTK.md" > "${claudeMdPath}"
      elif ! grep -qF "@RTK.md" "${claudeMdPath}"; then
        printf '\n@RTK.md\n' >> "${claudeMdPath}"
      fi
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

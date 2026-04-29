{
  inputs,
  pkgs,
  config,
  lib,
  ...
}: let
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
    };
  };

  settingsPath = "${config.home.homeDirectory}/.claude/settings.json";
  claudeMdPath = "${config.home.homeDirectory}/.claude/CLAUDE.md";
in {
  home = {
    packages = [
      pkgs.libnotify
    ];
    file = {
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
}

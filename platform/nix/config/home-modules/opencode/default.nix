{
  pkgs,
  config,
  lib,
  ...
}: let
  opencodeDir = "${config.home.homeDirectory}/.config/opencode";

  # The vendored set lives at home-modules/claude-code/skills/vendored. One
  # copy on disk, referenced from both agent CLIs' config trees, rather than
  # two trees that would have to be kept in step by hand.
  vendoredSkills = let
    dir = ../claude-code/skills/vendored;
  in
    lib.mapAttrs' (name: _:
      lib.nameValuePair ".config/opencode/skills/${name}" {source = dir + "/${name}";})
    (lib.filterAttrs (_: type: type == "directory") (lib.readDir dir));

  inherit (pkgs.pkgsUnstable) rtk;

  # Opencode's analogue of the Claude Code PreToolUse hook is the plugin
  # `tool.execute.before` event: a JS/TS module that may mutate
  # `output.args`. The rewrite protocol is the same as rtk-rewrite.sh's
  # (exit 0 + stdout = rewrite, non-zero = leave alone); auto-allow and
  # ask/deny handling are opencode's own via the `permission` config block,
  # not something a plugin hands back.
  rtkRewritePlugin = pkgs.writeText "rtk-rewrite.js" ''
    import { spawnSync } from "node:child_process"

    const RTK = "${rtk}/bin/rtk"

    export const RtkRewrite = async () => {
      return {
        "tool.execute.before": async (input, output) => {
          if (input.tool !== "bash") return
          const command = output?.args?.command
          if (typeof command !== "string" || command.length === 0) return

          const res = spawnSync(RTK, ["rewrite", command], {encoding: "utf-8"})
          if (res.error || res.status !== 0) return

          const rewritten = (res.stdout ?? "").trimEnd()
          if (rewritten.length > 0 && rewritten !== command) {
            output.args.command = rewritten
          }
        },
      }
    }
  '';

  # RTK awareness markdown for OpenCode agents. Identical to the Claude Code
  # copy except the rewriter framing (plugin, not hook) and the tool RTK's
  # `discover` reads.
  rtkAwarenessMd = pkgs.writeText "RTK.md" ''
    # RTK - Rust Token Killer

    **Usage**: Token-optimized CLI proxy (60-90% savings on dev operations)

    ## Meta Commands (always use rtk directly)

    ```bash
    rtk gain              # Show token savings analytics
    rtk gain --history    # Show command usage history with savings
    rtk discover          # Analyze OpenCode history for missed opportunities
    rtk proxy <cmd>       # Execute raw command without filtering (for debugging)
    ```

    ## Installation Verification

    ```bash
    rtk --version         # Should show: rtk X.Y.Z
    rtk gain              # Should work (not "command not found")
    which rtk             # Verify correct binary
    ```

    ## Plugin-Based Usage

    All other commands are automatically rewritten by the OpenCode plugin.

    **rtk compresses output.** When a count, an exact listing, or completeness
    matters, use `rtk proxy <cmd>` or the raw command; compressed output has
    misreported counts before.

    ## Long-Running Nix Commands

    **Never leave a background nix evaluation running when a session ends;
    stop it or wait for it.** An orphaned whole-tree eval OOM-killed a machine
    on 2026-08-19. Prefer bounded tools (`just check`) over raw whole-tree
    evaluation.

    ## Writing Style

    **Never use em dashes (`—`) in any output, commit messages, code comments, or
    documentation.** Rewrite the sentence, or use a comma, colon, parentheses, or a
    period instead.

    ## Code Comments

    **Every comment must pass the deepcomment test: could a competent reader
    derive it from the code? Then delete it.** Keep only why-not-what,
    contracts, and warnings, in 1-3 lines; a comment must never outweigh the
    code it describes. After writing or editing comments, run the
    `deepcomment` skill over the changes before committing.

    ## Shell

    **Commands most likely run in [nushell](https://www.nushell.sh/), not bash/POSIX sh.**
    Nushell is the configured default shell (`$env.SHELL = nu`). Its syntax differs
    from POSIX shells, so:

    - Set environment variables with `$env.VAR = "value"`, not `VAR=value` or `export`.
    - There are no `&&`/`||` shell operators; use `;` to sequence and nushell's own
      logic (`if`, `try`/`catch`) for conditionals.
    - Command substitution is `(cmd)`, not `$(cmd)` or backticks.
    - Glob/redirection and quoting rules differ from bash.
    - Prefer nushell pipelines and built-ins (`ls`, `where`, `get`, `to json`, ...)
      over POSIX idioms.
    - If you need POSIX syntax, invoke it explicitly, e.g. `bash -c "..."`.

    ## Version Control

    **Always use `jj` (Jujutsu) instead of `git` for all VCS operations.**

    ```bash
    jj status             # Working copy status
    jj log                # Commit log
    jj diff               # Show changes
    jj commit -m "msg"    # Finalize current change and start a new one on top
    jj describe -m "msg"  # Only to amend a description (does NOT finalize)
    jj bookmark set <name> -r @-  # Set bookmark (like a branch)
    jj git push           # Push to remote
    jj git fetch          # Fetch from remote
    ```

    **Always finish work by committing it. Prefer `jj commit -m "msg"` over
    `jj describe -m "msg"`.** `jj commit` sets the description and creates a new
    empty change on top in one step, so the finished work is actually committed and
    not left in the working copy. Only use `jj describe` to fix an existing change's
    description without finalizing it. Never end a task leaving completed changes
    uncommitted.

    **Never push directly to the default branch (`main`/`master`) unless the user explicitly asks.**
    Create a feature bookmark and push that instead:

    ```bash
    jj bookmark create my-feature -r @
    jj git push --bookmark my-feature
    ```
  '';

  # `$schema` is a literal key, so quote it for the JSON generator.
  opencodeSettingsJson = (pkgs.formats.json {}).generate "opencode.json" {
    "$schema" = "https://opencode.ai/config.json";
    # Load the Nix-managed rules file as a schema-level instruction, the
    # mechanism OpenCode supports where Claude Code uses an `@RTK.md` line
    # in CLAUDE.md.
    instructions = ["${opencodeDir}/RTK.md"];
  };

  # Claude Code gets its waiting/completion notifications from user hooks in
  # settings.json; OpenCode ships the same UX natively under `attention`.
  tuiSettingsJson = (pkgs.formats.json {}).generate "tui.json" {
    "$schema" = "https://opencode.ai/tui.json";
    attention = {
      enabled = true;
      notifications = true;
      sound = true;
    };
  };
in {
  home = {
    file =
      {
        ".config/opencode/plugins/rtk-rewrite.js".source = rtkRewritePlugin;
        ".config/opencode/RTK.md".source = rtkAwarenessMd;
      }
      # Skills, by reading the directory rather than listing them: they arrive
      # as a set from one upstream, so naming each here would be a second list
      # to keep in step with the first.
      // vendoredSkills;

    # Copy opencode.json and tui.json (not symlink) so OpenCode can write to
    # them. OpenCode has no AGENTS.md bootstrap step to mirror here: global
    # rules land via the `instructions` field above, and the directory's
    # user-editable AGENTS.md is optional by design.
    activation.opencodeSettings = lib.hm.dag.entryAfter ["writeBoundary"] ''
      install -Dm644 ${opencodeSettingsJson} ${opencodeDir}/opencode.json
      install -Dm644 ${tuiSettingsJson} ${opencodeDir}/tui.json
    '';
  };
}

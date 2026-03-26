# RTK (token-optimized commands)

- **Always prefix shell commands with `rtk`** to minimize token consumption. RTK filters and compresses command output before it reaches the LLM context, saving 60-90% tokens. Examples:
  - `rtk git status` instead of `git status`
  - `rtk cargo test` instead of `cargo test`
  - `rtk ls src/` instead of `ls src/`
  - `rtk grep "pattern" src/` instead of `grep "pattern" src/`
  - `rtk find "*.rs" .` instead of `find "*.rs" .`
  - `rtk read file.rs` instead of `cat file.rs`
  - `rtk docker ps` instead of `docker ps`
  - `rtk gh pr list` instead of `gh pr list`

- **Use `rtk` meta commands for analytics:**
  - `rtk gain` — show token savings statistics
  - `rtk discover` — find missed RTK opportunities
  - `rtk proxy <cmd>` — run raw command without filtering (for debugging)

## Version control rules

- **Use `jj` (Jujutsu) instead of `git` for all version control operations.** This project uses Jujutsu as its VCS. Common mappings:
  - `jj status` — show working copy status
  - `jj log` — show commit history
  - `jj diff` — show changes
  - `jj new` — create a new change (like finishing the current commit)
  - `jj commit -m "msg"` — set description and create a new change on top
  - `jj describe -m "msg"` — update the current change's description
  - `jj bookmark` — manage bookmarks (branches)
  - `jj git push` — push to remote
  - `jj git fetch` — fetch from remote

## Pushing rules

- **Always `jj git fetch` before pushing to avoid overwriting upstream changes.** Other branches may have been merged into `main` while you were working. Before moving the `main` bookmark and pushing, fetch first, then rebase or merge if needed. Never blindly `jj bookmark set main -r @- && jj git push` — this can silently discard commits merged upstream.

## Commit message rules

- **Follow `.commitlintrc.yml` for commit message format.** Before committing, read `.commitlintrc.yml` and ensure the commit message conforms to its rules.

- **Do NOT add `Co-Authored-By` lines to commit messages.** Never append co-author trailers (e.g. `Co-Authored-By: Claude ...`) to commits.

- **The `jj` wrapper runs git hooks automatically.** The `jj` binary is wrapped to run `pre-commit` hooks before `jj commit`, `jj new`, and `jj squash`, and `prepare-commit-msg` hooks when `-m`/`--message` is provided. If a hook fails, fix the issue and run the command again — do not try to bypass hooks.

## Pre-commit file review

- **Always review `jj status` for unintended files before committing.** jj auto-tracks all unignored files — there is no explicit staging step. Before every `jj commit` or `jj new`, check the file list and delete any test artifacts, temp files, or anything not intentionally part of the change. Be especially vigilant when running test suites that execute shell scripts, as they may create files in the working directory.

## Nix flake rules

- **Run any `jj` command (e.g. `jj status`) before Nix flake operations when you've created new files.** Nix flakes only see files tracked by git. In a jj colocated repo, jj automatically snapshots the working directory (updating the git index) on every `jj` command. Unlike plain git, you do NOT need to manually `git add` files — just ensure at least one `jj` command has run since creating the file.
- **Run `touch .envrc && direnv export json` after changing devenv modules or configs.** Files in `nix/modules/devenv/` and `nix/modules/devenv/configs/` are evaluated by devenv on shell entry. Changes to these files (e.g. `enterShell`, git-hooks, packages) won't take effect until you run `touch .envrc && direnv export json`. Note: `direnv reload` only touches `.envrc` and defers to a shell prompt hook that doesn't fire in non-interactive contexts. `direnv export json` directly triggers the full re-evaluation.

## Devenv config files

- **Root config files are symlinked or copied from `nix/modules/devenv/configs/` — never edit the root copies directly.** The devenv `enterShell` hook (in `nix/modules/devenv/configs/default.nix`) populates the project root on every shell entry. The mapping is:

  | Root file | Source | Method |
  |-|-|-|
  | `biome.jsonc` | `nix/modules/devenv/configs/biome-nix.jsonc` | symlink |
  | `deno.jsonc` | `nix/modules/devenv/configs/deno.jsonc` | symlink |
  | `lychee.toml` | `nix/modules/devenv/configs/lychee.toml` | symlink |
  | `rumdl.toml` | `nix/modules/devenv/configs/rumdl.toml` | symlink |
  | `typos.toml` | `nix/modules/devenv/configs/typos.toml` | symlink |
  | `.secretsignore` | `nix/modules/devenv/configs/secretsignore` | symlink |
  | `.commitlintrc.yml` | `nix/modules/devenv/configs/commitlintrc.nix` | generated (Nix derivation) — scope list is auto-derived from top-level directory names |
  | `.zed/settings.json` | `nix/modules/devenv/configs/zed/settings.jsonc` | copy |
  | `.rules` | `nix/modules/devenv/configs/zed/rules.md` | copy |
  | `.claude/rules/rules.md` | `nix/modules/devenv/configs/zed/rules.md` | copy |
  | `.tangled/workflows/*.yml` | `.tangled/workflows.ncl` | generated (Nickel export) — each top-level key becomes a workflow YAML file; also regenerated by pre-commit hook |

- **To update a config:** edit the source file in `nix/modules/devenv/configs/`, then run `touch .envrc && direnv export json` to regenerate/re-symlink the root copies. Symlinked files (most configs) point into the Nix store, so changes require a devenv re-evaluation. Copied files (`.rules`, `.claude/rules/rules.md`, `.zed/settings.json`) are overwritten on each shell entry.

- **To update tangled workflows:** edit `.tangled/workflows.ncl`, then the pre-commit hook will regenerate `.tangled/workflows/*.yml` automatically. Contracts are in `nickel/contracts/tangled-workflow/`. Never edit the YAML files directly.
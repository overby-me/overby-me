# RTK (token-optimized commands)

- **Always prefix shell commands with `rtk`** to minimize token consumption. RTK filters and compresses command output before it reaches the LLM context, saving 60-90% tokens. Examples:
  - `rtk jj status` instead of `jj status`
  - `rtk jj log` instead of `jj log`
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

- **Always prefer `jj commit -m "msg"` over `jj describe -m "msg"`.** `jj commit` sets the description and creates a new empty change on top in one step, which is almost always the desired workflow. Only use `jj describe` when you specifically need to amend the description of a change without creating a new one (e.g. fixing a typo in a commit message before pushing).

- **Never use `jj split` — it opens an interactive editor that an AI agent cannot control.** The command launches a TUI/editor for selecting changes, which will hang indefinitely in a non-interactive terminal. If you need to split changes, ask the user to do it manually.

## Pushing rules

- **Never push directly to the default branch (`main`/`master`) unless the user explicitly asks for it.** Instead, create a new bookmark for your changes and push that. Only move the `main` bookmark and push to `main` when the user says something like "push to main" or "commit and push". Example workflow:
  - `jj bookmark create my-feature -r @` — create a bookmark for the current change
  - `jj git push --bookmark my-feature` — push the feature bookmark
  - Let the user decide when to merge into `main`.

- **Always `jj git fetch` before pushing to avoid overwriting upstream changes.** Other branches may have been merged into `main` while you were working. Before moving the `main` bookmark and pushing, fetch first, then rebase or merge if needed. Never blindly `jj bookmark set main -r @- && jj git push` — this can silently discard commits merged upstream.

## Commit message rules

- **Follow `.commitlintrc.yml` for commit message format.** Before committing, read `.commitlintrc.yml` and ensure the commit message conforms to its rules.

- **Do NOT add `Co-Authored-By` lines to commit messages.** Never append co-author trailers (e.g. `Co-Authored-By: Claude ...`) to commits.

- **Always run `rtk jj diff --stat` before writing a commit message.** Base the commit message on the actual diff, not on memory of what was changed. This prevents misleading commit messages that reference changes from earlier (already-pushed) commits.

- **The `jj` wrapper runs git hooks automatically.** The `jj` binary is wrapped to run `pre-commit` hooks before `jj commit`, `jj new`, and `jj squash`, and `prepare-commit-msg` hooks when `-m`/`--message` is provided. If a hook fails, fix the issue and run the command again — do not try to bypass hooks.

## Pre-commit file review

- **Always review `jj status` for unintended files before committing.** jj auto-tracks all unignored files — there is no explicit staging step. Before every `jj commit` or `jj new`, check the file list and delete any test artifacts, temp files, or anything not intentionally part of the change. Be especially vigilant when running test suites that execute shell scripts, as they may create files in the working directory.

## Nix flake rules

- **Never run `nix flake check`: it gets OOM-killed in this repo.** The tree
  has thousands of check derivations (hundreds of them NixOS VM tests), and
  `nix flake check` instantiates all of them in a single evaluator whose peak
  memory exceeds 30 GiB. `--max-jobs`/`--no-build` do not help. Run
  `just check` instead: it invokes the bounded-memory `flake-check` tool
  (`nix/pkgs/flake-check.nix`), which evaluates attributes in memory-capped
  `nix-eval-jobs` workers and then builds only the uncached checks, one job at
  a time. Tune it via environment variables: `CHECK_FRAGMENT` (evaluate a
  different fragment, e.g. `CHECK_FRAGMENT=devShells.x86_64-linux`), `WORKERS`
  (default 2), `MAX_MEMORY_MB` (default 5120), `BATCH` (default 20), and
  `OUT_PATHS_FILE` (record built store paths for a cache push). To run a
  single known check, prefer a direct
  `nix build .#checks.x86_64-linux.<name>`, which is cheap and needs no
  special handling. Expect a full `just check` to take on the order of an
  hour; do not start it casually.
- **Run any `jj` command (e.g. `jj status`) before Nix flake operations when you've created new files.** Nix flakes only see files tracked by git. In a jj colocated repo, jj automatically snapshots the working directory (updating the git index) on every `jj` command. Unlike plain git, you do NOT need to manually `git add` files — just ensure at least one `jj` command has run since creating the file.
- **Run `touch .envrc && direnv export json` after changing devshell modules or configs.** Files in `nix/devshell/modules/` and `nix/devshell/modules/configs/` are evaluated on devshell entry. Changes to these files (e.g. the config `shellHook`, git-hooks, packages) won't take effect until you run `touch .envrc && direnv export json`. Note: `direnv reload` only touches `.envrc` and defers to a shell prompt hook that doesn't fire in non-interactive contexts. `direnv export json` directly triggers the full re-evaluation.

## Devshell config files

- **Root config files are copied from `nix/devshell/modules/configs/` — never edit the root copies directly.** The devshell config `shellHook` (in `nix/devshell/modules/configs/default.nix`) populates the project root on every shell entry. All files are copied (via `install -m 644`) as real, writable files rather than symlinked into the read-only Nix store, so tools and AI agents are never prompted for permission to follow symlinks into `/nix/store`. The mapping is:

  | Root file | Source | Method |
  |-|-|-|
  | `biome.jsonc` | `devshell/modules/configs/biome-nix.jsonc` | copy |
  | `deno.jsonc` | `devshell/modules/configs/deno.jsonc` | copy |
  | `lychee.toml` | `devshell/modules/configs/lychee.toml` | copy |
  | `rumdl.toml` | `devshell/modules/configs/rumdl.toml` | copy |
  | `typos.toml` | `devshell/modules/configs/typos.toml` | copy |
  | `.secretsignore` | `devshell/modules/configs/secretsignore` | copy |
  | `.commitlintrc.yml` | `devshell/modules/configs/commitlintrc.nix` | generated (Nix derivation) — scope list is auto-derived from top-level directory names; copied into place |
  | `.zed/settings.json` | `devshell/modules/configs/zed/settings.jsonc` | copy |
  | `.rules` | `devshell/modules/configs/ai-rules.md` | copy |
  | `.claude/rules/rules.md` | `devshell/modules/configs/ai-rules.md` | copy |
  | `.tangled/workflows/*.yml` | `.tangled/workflows.ncl` | generated (Nickel export) — each top-level key becomes a workflow YAML file; also regenerated by pre-commit hook |

- **To update a config:** edit the source file in `devshell/modules/configs/`, then run `touch .envrc && direnv export json` to regenerate and re-copy the root copies. All root copies are overwritten on each shell entry from their (possibly Nix-store) sources, so changes to generated sources (e.g. `.commitlintrc.yml`) require a devshell re-evaluation to refresh the copied result.

- **To update tangled workflows:** edit `.tangled/workflows.ncl`, then the pre-commit hook will regenerate `.tangled/workflows/*.yml` automatically. Contracts are in `nickel/contracts/tangled-workflow/`. Never edit the YAML files directly.

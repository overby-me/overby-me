# Git-hooks devshell module.
#
# Wraps the `git-hooks` (cachix/git-hooks.nix) input directly: `run` returns a
# derivation whose `.shellHook` installs `.pre-commit-config.yaml` at the git
# root at runtime (via `git rev-parse`), so no build-time working directory is
# needed. Run `touch .envrc && direnv export json` to apply changes.
{
  pkgs,
  lib,
  src,
  inputs,
  ...
}: let
  commitlintrc = import ./configs/commitlintrc.nix {inherit pkgs lib src;};
  preCommit = inputs.git-hooks.lib.${pkgs.stdenv.hostPlatform.system}.run {
    src = inputs.self;
    package = pkgs.prek;
    hooks = {
      denolint.enable = false;
      flake-checker = {
        enable = true;
        # flake-checker 0.2.11's hardcoded supported-branch list lags real
        # releases (does not yet know about nixos-26.05). Disable that one
        # check; outdated/owner checks still run.
        entry = "${pkgs.writeShellScript "flake-checker-allow-current-release" ''
          NIX_FLAKE_CHECKER_CHECK_SUPPORTED=false exec ${pkgs.flake-checker}/bin/flake-checker -f "$@"
        ''}";
      };
      biome = {
        enable = true;
        # web/wiki is a Rust/Dioxus/WASM app, not a JS/TS project. Its incidental
        # JS/CSS/JSON (service worker, theme + test scripts, lexicon docs) is
        # hand-authored to its own conventions and governed by the app's own gates
        # (rustfmt/clippy/cargo test/check-css-spacing.nu), not by biome.
        excludes = ["^web/wiki/"];
      };
      alejandra.enable = true;
      deadnix.enable = true;
      ripsecrets.enable = true;
      statix.enable = true;
      tombi-format = {
        enable = true;
        name = "tombi-format";
        entry = "${pkgs.tombi}/bin/tombi format --offline";
        files = "\\.toml$";
        pass_filenames = true;
      };
      typos = {
        enable = true;
        settings.configPath = "./nix/devshell/modules/configs/typos.toml";
        # Every typos hit in web/wiki was a false positive on technical content:
        # plural all-caps SQL keywords (a trailing lowercase "s" confuses the
        # tokenizer), percent-encoded UTF-8 test fixtures, and ported short
        # identifiers. Allow-listing those tokens in the shared typos.toml would
        # mask real typos monorepo-wide, so scope typos to skip the app instead
        # (its i18n strings were already excluded). deslop and lychee, which DO
        # find real issues here, stay enabled with tuned configs.
        excludes = ["^web/wiki/"];
      };
      nickel-format = {
        enable = true;
        name = "nickel-format";
        entry = "${pkgs.pkgsUnstable.nickel}/bin/nickel format";
        files = "\\.ncl$";
        pass_filenames = true;
      };
      tangled-workflows = {
        enable = true;
        name = "tangled-workflows";
        entry = "${pkgs.writeShellScript "tangled-workflows-generate" ''
          if ! echo "$@" | ${pkgs.gnugrep}/bin/grep -q '\.tangled/workflows\.ncl\|nickel/contracts/tangled-workflow/'; then
            exit 0
          fi
          mkdir -p .tangled/workflows
          for key in $(${pkgs.pkgsUnstable.nickel}/bin/nickel export --format yaml .tangled/workflows.ncl | ${pkgs.yq-go}/bin/yq 'keys | .[]'); do
            ${pkgs.pkgsUnstable.nickel}/bin/nickel export --format yaml .tangled/workflows.ncl \
              | ${pkgs.yq-go}/bin/yq ".$key" > ".tangled/workflows/$key.yml"
          done
          ${pkgs.git}/bin/git add .tangled/workflows/
        ''}";
        files = "\\.ncl$";
        pass_filenames = true;
      };
      mojo-format = {
        enable = true;
        name = "mojo-format";
        entry = "${pkgs.mojo}/bin/mojo format";
        files = "\\.mojo$";
        pass_filenames = true;
      };
      lychee = let
        lychee-changed-lines = pkgs.writeShellScriptBin "lychee-changed-lines" ''
          token=$(${pkgs.gh}/bin/gh auth token 2>/dev/null || true)
          lychee_cmd="${pkgs.lychee}/bin/lychee"
          if [ -n "$token" ]; then
            lychee_cmd="$lychee_cmd --github-token $token"
          fi

          # Extract only added lines from the staged diff of the given files
          changed_content=""
          for file in "$@"; do
            added=$(${pkgs.git}/bin/git diff --cached -U0 -- "$file" | ${pkgs.gnugrep}/bin/grep '^+' | ${pkgs.gnugrep}/bin/grep -v '^+++' | ${pkgs.gnused}/bin/sed 's/^+//')
            if [ -n "$added" ]; then
              changed_content="$changed_content
          $added"
            fi
          done

          if [ -z "$changed_content" ]; then
            exit 0
          fi

          echo "$changed_content" | $lychee_cmd -
        '';
      in {
        enable = true;
        package = lychee-changed-lines;
        entry = "${lychee-changed-lines}/bin/lychee-changed-lines";
      };
      rustfmt = {
        enable = true;
        entry = "${pkgs.writeShellScript "rustfmt-multi-project" ''
          # Determine which Cargo projects contain changed .rs files.
          # Arguments are the changed .rs file paths passed by pre-commit.
          changed_files=("$@")

          if [ ''${#changed_files[@]} -eq 0 ]; then
            exit 0
          fi

          # Find the nearest Cargo.toml for each changed file and collect unique project roots.
          declare -A project_roots
          for f in "''${changed_files[@]}"; do
            dir=$(dirname "$f")
            while [ "$dir" != "." ] && [ "$dir" != "/" ]; do
              if [ -f "$dir/Cargo.toml" ]; then
                project_roots["$dir"]=1
                break
              fi
              dir=$(dirname "$dir")
            done
            # Check current directory too
            if [ -f "Cargo.toml" ] && [ "$dir" = "." ]; then
              project_roots["."]=1
            fi
          done

          if [ ''${#project_roots[@]} -eq 0 ]; then
            exit 0
          fi

          # For each project root, walk up to find the workspace root (if any).
          declare -A fmt_targets
          for root in "''${!project_roots[@]}"; do
            ws_root=""
            check_dir="$root"
            while [ "$check_dir" != "." ] && [ "$check_dir" != "/" ]; do
              if [ -f "$check_dir/Cargo.toml" ] && ${pkgs.gnugrep}/bin/grep -q '^\[workspace\]' "$check_dir/Cargo.toml"; then
                ws_root="$check_dir"
              fi
              check_dir=$(dirname "$check_dir")
            done
            # Also check the repo root
            if [ -f "Cargo.toml" ] && ${pkgs.gnugrep}/bin/grep -q '^\[workspace\]' "Cargo.toml"; then
              ws_root="."
            fi

            if [ -n "$ws_root" ]; then
              # Use the workspace root; cargo fmt handles all members
              fmt_targets["$ws_root"]=1
            else
              # Standalone package
              fmt_targets["$root"]=1
            fi
          done

          pids=()
          for target in "''${!fmt_targets[@]}"; do
            manifest="$target/Cargo.toml"
            if ${pkgs.gnugrep}/bin/grep -q '^\[workspace\]' "$manifest"; then
              # Workspace root: use --all to format all members
              echo "Running cargo fmt --all for workspace $manifest"
              ${pkgs.cargo}/bin/cargo fmt --manifest-path "$manifest" --all &
              pids+=($!)
            else
              echo "Running cargo fmt for $manifest"
              ${pkgs.cargo}/bin/cargo fmt --manifest-path "$manifest" &
              pids+=($!)
            fi
          done
          exit_code=0
          for pid in "''${pids[@]}"; do
            if ! wait "$pid"; then
              exit_code=1
            fi
          done
          exit $exit_code
        ''}";
        pass_filenames = true;
      };
      clippy = {
        enable = true;
        entry = "${pkgs.writeShellScript "clippy-multi-project" ''
          # Determine which Cargo projects contain changed .rs files.
          # Arguments are the changed .rs file paths passed by pre-commit.
          changed_files=("$@")

          if [ ''${#changed_files[@]} -eq 0 ]; then
            exit 0
          fi

          # Find the nearest Cargo.toml for each changed file and collect unique project roots.
          declare -A project_roots
          for f in "''${changed_files[@]}"; do
            dir=$(dirname "$f")
            while [ "$dir" != "." ] && [ "$dir" != "/" ]; do
              if [ -f "$dir/Cargo.toml" ]; then
                project_roots["$dir"]=1
                break
              fi
              dir=$(dirname "$dir")
            done
            # Check current directory too
            if [ -f "Cargo.toml" ] && [ "$dir" = "." ]; then
              project_roots["."]=1
            fi
          done

          if [ ''${#project_roots[@]} -eq 0 ]; then
            exit 0
          fi

          # For each project root, walk up to find the workspace root (if any).
          # Separate into workspace roots and standalone packages.
          declare -A workspace_roots
          declare -A standalone_roots
          for root in "''${!project_roots[@]}"; do
            ws_root=""
            check_dir="$root"
            while [ "$check_dir" != "." ] && [ "$check_dir" != "/" ]; do
              if [ -f "$check_dir/Cargo.toml" ] && ${pkgs.gnugrep}/bin/grep -q '^\[workspace\]' "$check_dir/Cargo.toml"; then
                ws_root="$check_dir"
              fi
              check_dir=$(dirname "$check_dir")
            done
            # Also check the repo root
            if [ -f "Cargo.toml" ] && ${pkgs.gnugrep}/bin/grep -q '^\[workspace\]' "Cargo.toml"; then
              ws_root="."
            fi

            if [ -n "$ws_root" ]; then
              workspace_roots["$ws_root"]=1
            else
              standalone_roots["$root"]=1
            fi
          done

          pids=()

          # Run clippy for affected workspaces
          for ws_dir in "''${!workspace_roots[@]}"; do
            manifest="$ws_dir/Cargo.toml"
            echo "Running cargo clippy --workspace for $manifest"
            ${pkgs.cargo}/bin/cargo clippy --manifest-path "$manifest" --workspace -- -D warnings &
            pids+=($!)
          done

          # Run clippy for standalone packages (not part of a workspace)
          for pkg_dir in "''${!standalone_roots[@]}"; do
            manifest="$pkg_dir/Cargo.toml"
            if ! ${pkgs.gnugrep}/bin/grep -q '^\[package\]' "$manifest"; then
              continue
            fi
            echo "Running cargo clippy for $manifest"
            ${pkgs.cargo}/bin/cargo clippy --manifest-path "$manifest" -- -D warnings &
            pids+=($!)
          done

          exit_code=0
          for pid in "''${pids[@]}"; do
            if ! wait "$pid"; then
              exit_code=1
            fi
          done
          exit $exit_code
        ''}";
        pass_filenames = true;
      };
      deslop = {
        enable = true;
        name = "deslop";
        entry = "${pkgs.writeShellScript "deslop-precommit" ''
          exit_code=0
          # Collect unique scan roots: walk up from each file to find a
          # .deslop.toml; if found, scan the containing directory (once).
          # Files without a .deslop.toml ancestor are scanned individually.
          declare -A seen_dirs
          individual_files=()
          for file in "$@"; do
            dir="$(dirname "$file")"
            found=""
            d="$dir"
            while true; do
              if [ -f "$d/.deslop.toml" ]; then
                found="$d"
                break
              fi
              parent="$(dirname "$d")"
              if [ "$parent" = "$d" ]; then
                break
              fi
              d="$parent"
            done
            if [ -n "$found" ]; then
              seen_dirs["$found"]=1
            else
              individual_files+=("$file")
            fi
          done
          for d in "''${!seen_dirs[@]}"; do
            if ! ${pkgs.deslop}/bin/deslop scan "$d"; then
              exit_code=1
            fi
          done
          for file in "''${individual_files[@]}"; do
            if ! ${pkgs.deslop}/bin/deslop scan "$file"; then
              exit_code=1
            fi
          done
          exit $exit_code
        ''}";
        files = "\\.(rs|go|py)$";
        pass_filenames = true;
      };
      rumdl = {
        enable = true;
        entry = "${pkgs.rumdl}/bin/rumdl fmt";
      };
      mktoc = {
        enable = false;
        package = pkgs.mktoc;
        name = "pre-commit-mktoc";
        entry = "${pkgs.mktoc}/bin/mktoc";
        files = "README\\.md$";
      };
      nil = {
        enable = true;
        entry = "${pkgs.writeShellScript "precommit-nil" ''
          errors=false
          echo Checking: $@
          for file in $(echo "$@"); do
            ${pkgs.nil}/bin/nil diagnostics --deny-warnings "$file"
            exit_code=$?

            if [[ $exit_code -ne 0 ]]; then
              echo \"$file\" failed with exit code: $exit_code
              errors=true
            fi
          done
          if [[ $errors == true ]]; then
            exit 1
          fi
        ''}";
      };
      commitlint-rs = {
        enable = true;
        package = pkgs.commitlint-rs;
        name = "prepare-commit-msg-commitlint-rs";
        entry = "${pkgs.commitlint-rs}/bin/commitlint --config ${commitlintrc} --edit";
        stages = ["prepare-commit-msg"];
      };
    };
  };
in {
  config = {
    packages = [pkgs.prek] ++ preCommit.enabledPackages;
    inherit (preCommit) shellHook;
  };
}

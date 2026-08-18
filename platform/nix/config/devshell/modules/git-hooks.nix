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
        # apps/wiki is deliberately not excluded. It used to be, and the hook
        # then disagreed with the check: `nix fmt` maps *.js to biome for every
        # path in the tree, and biome formats a file handed to it explicitly
        # whatever `files.includes` says. So a new .js under apps/wiki passed
        # every local gate and failed CI, which is how assets/heic-worker.js
        # reached main unformatted. The two have to agree; this direction finds
        # out before pushing rather than after.
      };
      alejandra.enable = true;
      deadnix.enable = true;
      ripsecrets.enable = true;
      statix.enable = true;
      ast-grep = {
        enable = true;
        name = "ast-grep";
        # Same two trees the clippy hook leaves alone: upstream code at a
        # pinned commit, and the ports PORTING.md governs. The vendored ooxml
        # parsers carry ignore annotations from their own upstream ruleset,
        # which would otherwise be reported here as unused.
        excludes = ["^apps/wiki/vendor/" "^safety/oxidized/"];
        # Structural rules that statix and clippy do not cover, in
        # dev/ast-grep/rules. Scans whole files rather than the changed hunk,
        # because a rule's ignore globs are how its exceptions are recorded and
        # those are path-based.
        entry = "${pkgs.ast-grep}/bin/ast-grep scan";
        files = "\\.(nix|rs)$";
        pass_filenames = true;
      };
      tombi-format = {
        enable = true;
        name = "tombi-format";
        entry = "${pkgs.tombi}/bin/tombi format --offline";
        files = "\\.toml$";
        pass_filenames = true;
      };
      typos = {
        enable = true;
        settings.configPath = "./platform/nix/config/devshell/modules/configs/typos.toml";
        # Every typos hit in apps/wiki was a false positive on technical content:
        # plural all-caps SQL keywords, percent-encoded UTF-8 fixtures, ported
        # short identifiers. Allow-listing those in the shared typos.toml would
        # mask real typos monorepo-wide, so skip the app instead
        # (its i18n strings were already excluded). deslop and lychee, which DO
        # find real issues here, stay enabled with tuned configs.
        excludes = ["^apps/wiki/"];
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
          if ! echo "$@" | ${pkgs.gnugrep}/bin/grep -q '\.tangled/workflows\.ncl\|dev/nickel/contracts/tangled-workflow/'; then
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
      clippy = let
        # Lints beyond clippy's default set, enabled centrally so every
        # workspace gets them without a [lints] block of its own. Chosen from a
        # measurement over the ten native workspaces (2556 findings across 57
        # candidates, lib and bin targets only, which is what this hook checks).
        #
        # The first group never fired anywhere, so it costs nothing today and
        # keeps the pattern from arriving. Seven of them are rules the
        # .deslop.toml files suppress by hand in up to nine projects.
        unusedToday = [
          "dbg_macro"
          "todo"
          "unimplemented"
          "unreachable"
          "panic"
          "panic_in_result_fn"
          "get_unwrap"
          "try_err"
          "wildcard_imports"
          "float_cmp"
          "lossy_float_literal"
          "fn_params_excessive_bools"
          "mutex_atomic"
          "rc_mutex"
          "rc_buffer"
          "large_futures"
          "large_stack_frames"
          "cast_ptr_alignment"
          "ref_as_ptr"
          "transmute_undefined_repr"
        ];
        # The second group fires under twenty times each, so the whole set is
        # about 150 findings to answer across the repo. `unwrap_used` is
        # seventeen of them: deslop's headline rule, which its own
        # implementation misses entirely.
        worthFixing = [
          "unwrap_used"
          "expect_used"
          "unwrap_in_result"
          "multiple_unsafe_ops_per_block"
          "ptr_as_ptr"
          "mem_forget"
          "cast_precision_loss"
          "needless_pass_by_value"
          "exit"
          "same_name_method"
          "verbose_file_reads"
          "missing_panics_doc"
          "implicit_hasher"
          "too_many_lines"
        ];
        # Four were tried and dropped. unused_async fired eleven times, every
        # one on a function that has to be async to exist: an axum handler
        # satisfying the Handler trait, or an arm of a dispatch table whose
        # other arms await. mod_module_files prefers foo.rs over foo/mod.rs,
        # and this tree has chosen mod.rs.
        # struct_excessive_bools reads a run of
        # booleans as a state machine wanting an enum, which is wrong for a
        # wire format: H.265's Pps carries 21 one-bit flags because the
        # specification says so. missing_asserts_for_indexing fired four times,
        # every one where the bound was already checked - by `windows(2)`, or by
        # an early return on `len()` - in a form the lint cannot see.
        #
        # Deliberately left off. arithmetic_side_effects (552) and
        # indexing_slicing (373) want panic-free code, which is not what a tree
        # of parsers and video decoders is; str_to_string (345) is a style
        # preference; print_stdout and print_stderr (339) fire on the CLIs whose
        # job is printing; missing_errors_doc (228) is documentation debt, not a
        # defect. Also staged out for now: the cast and FFI family
        # (cast_possible_truncation 90, cast_sign_loss 37, cast_possible_wrap
        # 36, borrow_as_ptr 22, undocumented_unsafe_blocks 33) and
        # allow_attributes_without_reason (47), which are worth having but are
        # 265 findings concentrated in the FFI shims.
        lintFlags = lib.concatMapStringsSep " " (l: "-W clippy::${l}") (unusedToday ++ worthFixing);
      in {
        enable = true;
        # safety/fe-c pins a nightly toolchain and its fe-c-driver crate uses
        # `#![feature(rustc_private)]`, which the stable `pkgs.cargo` here
        # cannot build. Its clippy runs on the pinned nightly via the
        # `fe-c-clippy` flake check (--all-features) instead.
        #
        # apps/wiki/vendor is upstream code held at a pinned commit (see
        # vendor/ooxml/README.md), so its lints are not ours to answer and
        # fixing them is divergence we would have to re-apply on every
        # re-vendor. It has never been clean; it stayed invisible only because
        # nothing touched those files. `nix fmt` reaching them is what put the
        # crate in this hook's scope, since the hook lints whole crates rather
        # than the changed file.
        excludes = ["^safety/fe-c/" "^apps/wiki/vendor/"];
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
            ${pkgs.cargo}/bin/cargo clippy --manifest-path "$manifest" --workspace -- ${lintFlags} -D warnings &
            pids+=($!)
          done

          # Run clippy for standalone packages (not part of a workspace)
          for pkg_dir in "''${!standalone_roots[@]}"; do
            manifest="$pkg_dir/Cargo.toml"
            if ! ${pkgs.gnugrep}/bin/grep -q '^\[package\]' "$manifest"; then
              continue
            fi
            echo "Running cargo clippy for $manifest"
            ${pkgs.cargo}/bin/cargo clippy --manifest-path "$manifest" -- ${lintFlags} -D warnings &
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
      cargo-profiles = {
        enable = true;
        name = "cargo-profiles";
        # Release-profile policy, which lives in the manifest rather than the
        # code: clippy lints Rust, and TOML is not an ast-grep language.
        entry = "${pkgs.nushell}/bin/nu ${src}/dev/scripts/check-cargo-profiles.nu";
        files = "Cargo\\.toml$";
        pass_filenames = false;
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

# Tangled workflow definitions for overby.me
#
# Generate YAML: nickel export --format yaml .tangled/workflows.ncl
# Each top-level key becomes .tangled/workflows/<key>.yml
let { WorkflowConfig, .. } = import "../nickel/contracts/tangled-workflow/workflow.ncl" in

# Experimental Nix features this flake needs (its rust build system relies on
# dynamic/recursive derivations). The microVM image's nix does not enable them
# and the engine exposes no nix.settings knob, so pass them as an explicit CLI
# flag on every nix invocation — parsed by the nix process directly, regardless
# of how the engine injects environment variables.
let nixFeatures = "nix-command flakes ca-derivations dynamic-derivations recursive-nix" in

{
  flake-check | WorkflowConfig = {
    when = [{ event = ["push"], branch = "**" }],
    engine = "microvm",
    image = "nixos",
    dependencies = ["git"],
    steps = [
      {
        name = "Build Cachix",
        command = m%%"
          nix build --extra-experimental-features "%%{nixFeatures}" --max-jobs auto .#rust-cachix
          export PATH="$(readlink -f result)/bin:$PATH"
          cachix --version
        "%%,
      },
      {
        name = "Setup Cachix",
        command = m%%"
          export PATH="$(readlink -f result)/bin:$PATH"
          mkdir -p $HOME/.config/nix
          cachix use overby-me
        "%%,
      },
      {
        name = "Nix flake check",
        # Not `nix flake check`: it instantiates all ~3265 checks in a single
        # evaluator, which keeps the whole value graph reachable and peaks past
        # 30 GiB before any build starts. `--max-jobs` caps build concurrency,
        # not evaluation, so it does not help. `.#flake-check` evaluates in
        # memory-capped nix-eval-jobs workers, skips derivations cachix already
        # has, and realises the rest one build job at a time.
        #
        # NIX_CONFIG is exported by this shell rather than injected by the
        # engine, so every nix process the script spawns inherits the features.
        #
        # The check outputs are pushed here rather than in "Push to Cachix":
        # that step only runs on success and only uploads `nix flake archive`
        # paths (the flake source and its inputs), never build results. Without
        # this every run rebuilds all checks from scratch. Pushing regardless of
        # the exit status means a partial run still makes the next one cheaper.
        command = m%%"
          echo "=== runs as: $(id -un); /etc/nix/nix.conf: ==="
          cat /etc/nix/nix.conf 2>/dev/null || echo "(no /etc/nix/nix.conf)"
          ulimit -n 65536
          printf '{"PWD":"%s"}' "$PWD" > .env
          export PATH="$(readlink -f result)/bin:$PATH"
          export NIX_CONFIG="experimental-features = %%{nixFeatures}"
          export OUT_PATHS_FILE="$PWD/check-outputs.txt"
          rc=0
          nix run --extra-experimental-features "%%{nixFeatures}" .#flake-check -- --override-input env path:$(realpath $PWD/.env) || rc=$?
          if [ -s "$OUT_PATHS_FILE" ]; then
            cachix push overby-me < "$OUT_PATHS_FILE" || echo "cachix push failed; continuing"
          fi
          exit $rc
        "%%,
      },
      {
        name = "Push to Cachix",
        command = m%%"
          export PATH="$(readlink -f result)/bin:$PATH"
          nix flake archive --extra-experimental-features "%%{nixFeatures}" --json --override-input env path:$(realpath $PWD/.env) | nix run --extra-experimental-features "%%{nixFeatures}" nixpkgs#jq -- -r '.path,(.inputs|to_entries[].value.path)' | cachix push overby-me
        "%%,
      },
    ],
  },

  mirror | WorkflowConfig = {
    when = [{ event = ["push"], branch = "main" }],
    engine = "microvm",
    image = "nixos",
    dependencies = ["git"],
    steps = [
      {
        name = "Fetch full main history",
        command = m%%"
          git fetch --unshallow origin main
        "%%,
      },
      {
        name = "Mirror to GitHub",
        command = m%%"
          git remote add github https://x-access-token:${GITHUB_MIRROR_TOKEN}@github.com/overby-me/overby-me.git 2>/dev/null || true
          git push --force github HEAD:refs/heads/main
        "%%,
      },
      {
        name = "Mirror to Codeberg",
        command = m%%"
          git remote add codeberg https://x-access-token:${CODEBERG_MIRROR_TOKEN}@codeberg.org/overby-me/overby-me.git 2>/dev/null || true
          git push --force codeberg HEAD:refs/heads/main
        "%%,
      },
    ],
  },
}

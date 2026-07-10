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
        # `cachix use` only adds overby-me, which holds this repo's own build
        # outputs. It does not add cache.nixos.org, and the microVM image has no
        # substituters of its own, so nixpkgs itself cannot be substituted. That
        # forces a from-source rebuild of nixpkgs' bootstrap, whose
        # minimal-bootstrap/tinycc source is a repo.or.cz cgit snapshot with a
        # rotted, non-reproducible hash, so the build dies on a FOD hash
        # mismatch. cache.nixos.org has that output signed and cached; add it so
        # the bootstrap substitutes instead of rebuilding. Appended to the same
        # nix.conf cachix writes, so it persists into the flake-check step.
        command = m%%"
          export PATH="$(readlink -f result)/bin:$PATH"
          mkdir -p $HOME/.config/nix
          cachix use overby-me
          echo "extra-substituters = https://cache.nixos.org" >> $HOME/.config/nix/nix.conf
          echo "extra-trusted-public-keys = cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY=" >> $HOME/.config/nix/nix.conf
        "%%,
      },
      {
        name = "Nix flake check",
        # The full check set does not obviously fit this CI's microVM (~2 GiB
        # RAM, 5-minute step timeout, 686 nested-QEMU VM tests). This attempts
        # it anyway and relies on tangled's cache to carry build progress across
        # runs: nix-eval-jobs streams each check, --check-cache-status skips what
        # the cache already has, and each build is uploaded as it completes, so a
        # run that is killed still advances the next one. WORKERS=1 and a low
        # per-worker memory cap keep the evaluator inside the microVM's RAM
        # rather than repeating the 2-worker OOM.
        #
        # The known risk: evaluation is not cached by tangled's store cache, so
        # if a full eval cannot finish within the 5-minute step it is killed
        # before building and never converges. If that happens, scope back to
        # `.#checks.x86_64-linux.formatting`, which does fit. Local `just check`
        # runs this same tool on a real machine.
        command = m%%"
          echo "=== runs as: $(id -un); /etc/nix/nix.conf: ==="
          cat /etc/nix/nix.conf 2>/dev/null || echo "(no /etc/nix/nix.conf)"
          ulimit -n 65536
          printf '{"PWD":"%s"}' "$PWD" > .env
          export NIX_CONFIG="experimental-features = %%{nixFeatures}"
          export WORKERS=1
          export MAX_MEMORY_MB=1024
          export BATCH=5
          nix run --extra-experimental-features "%%{nixFeatures}" .#flake-check -- --override-input env path:$(realpath $PWD/.env)
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

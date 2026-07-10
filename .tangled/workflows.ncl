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
    # Trusted binary caches. The microVM image ships no substituters, and its CI
    # user is untrusted, so a client-set `substituters` is ignored. Declaring
    # them here makes the spindle configure them system-side (trusted): nixpkgs
    # substitutes from cache.nixos.org instead of a from-source bootstrap, and
    # this repo's own outputs substitute from overby-me instead of rebuilding.
    caches = {
      "https://cache.nixos.org" = "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY=",
      "https://overby-me.cachix.org" = "overby-me.cachix.org-1:dU7qOj5u97QZz98nqnh+Nwait6c+2d2Eq0KTOAXTyp4=",
      "https://nix-community.cachix.org" = "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs=",
      "https://zed.cachix.org" = "zed.cachix.org-1:/pHQ6dpMsAZk2DiP4WCL0p9YDNKWj2Q5FL20bNmw1cU=",
    },
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
        # Substituters are declared by the workflow-level `caches` field above
        # (system-trusted), so nothing is configured here. cachix is still set up
        # for the "Push to Cachix" step's upload of the flake inputs.
        command = m%%"
          export PATH="$(readlink -f result)/bin:$PATH"
          mkdir -p $HOME/.config/nix
          cachix use overby-me
        "%%,
      },
      {
        name = "Nix flake check (formatting + packages)",
        # Scoped to formatting and the packages. The full check set does not fit
        # this CI's microVM (~2 GiB RAM, ~5-minute step timeout): evaluating all
        # 3266 checks thrashes a memory-capped worker and the 686 nested-QEMU VM
        # tests cannot build here. Formatting and packages do fit — packages
        # evaluate within the RAM budget, and with the caches above they
        # substitute from cache.nixos.org and overby-me rather than building.
        #
        # nix-fast-build (nix-eval-jobs + one build job at a time) evaluates in a
        # single memory-capped worker and --skip-cached skips anything a cache
        # already has, so only genuinely new packages build. Local `just check`
        # runs the full `.#flake-check` on a real machine.
        command = m%%"
          echo "=== runs as: $(id -un); /etc/nix/nix.conf: ==="
          cat /etc/nix/nix.conf 2>/dev/null || echo "(no /etc/nix/nix.conf)"
          ulimit -n 65536
          printf '{"PWD":"%s"}' "$PWD" > .env
          export NIX_CONFIG="experimental-features = %%{nixFeatures}"
          for fragment in "checks.x86_64-linux.formatting" "packages.x86_64-linux"; do
            nix run --extra-experimental-features "%%{nixFeatures}" nixpkgs#nix-fast-build -- \
              --flake ".#$fragment" --skip-cached --no-nom \
              --eval-workers 1 --eval-max-memory-size 1024 -j 1 \
              --override-input env "path:$(realpath $PWD/.env)"
          done
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

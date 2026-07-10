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
        # The microVM image ships no public substituters, so add cache.nixos.org
        # for the check step; without it nixpkgs' bootstrap rebuilds from source
        # and dies on the rotted minimal-bootstrap/tinycc snapshot. The
        # workflow-level `caches` field would be the documented way to do this,
        # but this knot's spindle does not honour it (the caches stay untrusted
        # and it broke substitution in the Build Cachix step), so append to the
        # nix.conf cachix writes instead. overby-me cannot be made readable this
        # way (client substituters are ignored for the untrusted CI user), which
        # is why CI cannot substitute this repo's own packages.
        command = m%%"
          export PATH="$(readlink -f result)/bin:$PATH"
          mkdir -p $HOME/.config/nix
          cachix use overby-me
          echo "extra-substituters = https://cache.nixos.org" >> $HOME/.config/nix/nix.conf
          echo "extra-trusted-public-keys = cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY=" >> $HOME/.config/nix/nix.conf
        "%%,
      },
      {
        name = "Nix flake check (formatting)",
        # Scoped to the formatting check. The full check set does not fit this
        # CI's microVM (~2 GiB RAM, ~5-minute step timeout): evaluating all 3266
        # checks thrashes a memory-capped worker, and the 686 nested-QEMU VM
        # tests cannot build here. Packages do not fit either: they live only in
        # overby-me, which cannot be made a trusted substituter for the untrusted
        # CI user, so they would rebuild from source. checks.formatting is the
        # one check that fits — nix forces only that attribute and its formatter
        # tools substitute from cache.nixos.org. Local `just check` runs the full
        # `.#flake-check` on a real machine.
        command = m%%"
          echo "=== runs as: $(id -un); /etc/nix/nix.conf: ==="
          cat /etc/nix/nix.conf 2>/dev/null || echo "(no /etc/nix/nix.conf)"
          ulimit -n 65536
          printf '{"PWD":"%s"}' "$PWD" > .env
          export NIX_CONFIG="experimental-features = %%{nixFeatures}"
          nix build --extra-experimental-features "%%{nixFeatures}" --max-jobs 1 --cores 2 \
            --no-link --print-build-logs .#checks.x86_64-linux.formatting \
            --override-input env path:$(realpath $PWD/.env)
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

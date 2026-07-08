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
        command = m%%"
          echo "=== runs as: $(id -un); /etc/nix/nix.conf: ==="
          cat /etc/nix/nix.conf 2>/dev/null || echo "(no /etc/nix/nix.conf)"
          ulimit -n 65536
          printf '{"PWD":"%s"}' "$PWD" > .env
          nix flake check --keep-going --extra-experimental-features "%%{nixFeatures}" --max-jobs 1 --cores 2 --override-input env path:$(realpath $PWD/.env)
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

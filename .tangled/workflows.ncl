# Tangled workflow definitions for overby.me
#
# Generate YAML: nickel export --format yaml .tangled/workflows.ncl
# Each top-level key becomes .tangled/workflows/<key>.yml
let { WorkflowConfig, .. } = import "../nickel/contracts/tangled-workflow/workflow.ncl" in

{
  flake-check | WorkflowConfig = {
    when = [{ event = ["push"], branch = "**" }],
    engine = "microvm",
    image = "nixos",
    dependencies = ["git"],
    # The microVM (NixOS) image's nix has none of the experimental features
    # this flake needs, and the engine only injects per-step `environment`,
    # not a workflow-level one. Rather than rely on env injection, each nix
    # step exports NIX_CONFIG inline so the evaluating `nix` process always
    # sees the features (notably dynamic-derivations) and max-jobs.
    steps = [
      {
        name = "Build Cachix",
        command = m%%"
          export NIX_CONFIG="experimental-features = nix-command flakes ca-derivations dynamic-derivations recursive-nix
          max-jobs = auto"
          nix build .#rust-cachix
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
          export NIX_CONFIG="experimental-features = nix-command flakes ca-derivations dynamic-derivations recursive-nix
          max-jobs = auto"
          ulimit -n 65536
          printf '{"PWD":"%s"}' "$PWD" > .env
          nix flake check --max-jobs 1 --override-input env path:$(realpath $PWD/.env)
        "%%,
      },
      {
        name = "Push to Cachix",
        command = m%%"
          export NIX_CONFIG="experimental-features = nix-command flakes ca-derivations dynamic-derivations recursive-nix
          max-jobs = auto"
          export PATH="$(readlink -f result)/bin:$PATH"
          nix flake archive --json --override-input env path:$(realpath $PWD/.env) | nix run nixpkgs#jq -- -r '.path,(.inputs|to_entries[].value.path)' | cachix push overby-me
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

# Tangled workflow definitions for overby.me
#
# Generate YAML: nickel export --format yaml .tangled/workflows.ncl
# Each top-level key becomes .tangled/workflows/<key>.yml
let { WorkflowConfig, .. } = import "../nickel/contracts/tangled-workflow/workflow.ncl" in

{
  flake-check | WorkflowConfig = {
    when = [{ event = ["push"], branch = "**" }],
    engine = "nix",
    environment = { CACHIX_NAME = "overby-me" },
    steps = [
      {
        name = "Build Cachix",
        command = m%%"
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
          echo "experimental-features = nix-command flakes ca-derivations dynamic-derivations recursive-nix" > $HOME/.config/nix/nix.conf
          echo "max-jobs = auto" >> $HOME/.config/nix/nix.conf
          cachix use $CACHIX_NAME
        "%%,
      },
      {
        name = "Nix flake check",
        command = m%%"
          ulimit -n 65536
          printf '{"PWD":"%s"}' "$PWD" > .env
          nix flake check --max-jobs 1 --override-input env path:$(realpath $PWD/.env)
        "%%,
      },
      {
        name = "Push to Cachix",
        command = m%%"
          export PATH="$(readlink -f result)/bin:$PATH"
          nix flake archive --json --override-input env path:$(realpath $PWD/.env) | nix run nixpkgs#jq -- -r '.path,(.inputs|to_entries[].value.path)' | cachix push $CACHIX_NAME
        "%%,
      },
    ],
  },

  mirror | WorkflowConfig = {
    when = [{ event = ["push"], branch = "main" }],
    engine = "nix",
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

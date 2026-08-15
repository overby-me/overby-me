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
        name = "Nix flake check (formatting)",
        # Scoped to the formatting check, and nothing else. The microVM (~2 GiB
        # RAM) cannot do more: the full 3266-check set thrashes a memory-capped
        # evaluator and its 686 nested-QEMU VM tests cannot build here; packages
        # live only in overby-me, which cannot be made a trusted substituter for
        # this untrusted CI user, so they would rebuild from source. Even the old
        # `nix build .#rust-cachix` cache-setup step was fatal — copying cachix's
        # large closure took 14 minutes and crashed the VM — so cachix is dropped
        # entirely (tangled caches build outputs itself; no manual push needed).
        #
        # checks.formatting is the one check that fits: nix forces only that
        # attribute and its formatter tools substitute from cache.nixos.org.
        # cache.nixos.org is added to nix.conf here because the image ships no
        # public substituter; without it nixpkgs rebuilds from source and dies on
        # the rotted minimal-bootstrap/tinycc snapshot. Local `just check` runs
        # the full `.#flake-check` on a real machine.
        command = m%%"
          echo "=== runs as: $(id -un) ==="
          ulimit -n 65536
          mkdir -p $HOME/.config/nix
          echo "extra-substituters = https://cache.nixos.org" >> $HOME/.config/nix/nix.conf
          echo "extra-trusted-public-keys = cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY=" >> $HOME/.config/nix/nix.conf
          printf '{"PWD":"%s"}' "$PWD" > .env
          export NIX_CONFIG="experimental-features = %%{nixFeatures}"
          nix build --extra-experimental-features "%%{nixFeatures}" --max-jobs 1 --cores 2 \
            --no-link --print-build-logs .#checks.x86_64-linux.formatting \
            --override-input env path:$(realpath $PWD/.env)
        "%%,
      },
    ],
  },

  publish | WorkflowConfig = {
    # Spindle has no scheduled trigger: the only events are push,
    # pull_request and manual. Running on every push to main is fresher than
    # the hourly cadence a cron would give, and the script skips a project
    # whose filtered SHA already matches the remote, so a push that touches
    # nothing published costs two ls-remote calls.
    # TEMPORARY: the `josh` entry exists only to exercise this pipeline before
    # it reaches main. Remove it once the first CI run has been observed.
    when = [
      { event = ["push"], branch = "main" },
      { event = ["push"], branch = "josh" },
      { event = ["manual"] },
    ],
    engine = "microvm",
    image = "nixos",
    dependencies = ["git", "openssh", "nushell", "josh"],
    steps = [
      {
        # Tangled push is SSH-only, so the whole pipeline depends on the
        # microVM being allowed to open port 22 to the knot. Probe it first
        # and never fail on it, so a networking regression is legible in the
        # log instead of surfacing as an opaque push failure below.
        name = "Probe knot SSH reachability",
        command = m%%"
          ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
              -o ConnectTimeout=15 -T git@tangled.org || true
        "%%,
      },
      {
        # The default clone is depth 1; josh filters the whole history.
        name = "Fetch full main history",
        command = m%%"
          git fetch --unshallow origin main || git fetch origin main
        "%%,
      },
      {
        # The key belongs to a bot account that is a collaborator on the
        # published repos, not to the repo owner: a leak must not be able to
        # write to this monorepo.
        name = "Install the publish key",
        command = m%%"
          mkdir -p $HOME/.ssh
          printf '%s\n' "${TANGLED_PUBLISH_KEY}" > $HOME/.ssh/publish_key
          chmod 600 $HOME/.ssh/publish_key
        "%%,
      },
      {
        name = "Publish subprojects",
        command = m%%"
          nu tangled/publish/publish.nu \
            --source "$PWD" \
            --work-dir "$PWD/.publish-cache" \
            --ssh-user agent.overby.me \
            --ssh-key "$HOME/.ssh/publish_key"
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

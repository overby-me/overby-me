lint *hook:
    prek run {{ if hook == "" { "--all-files" } else { hook + " --all-files" } }}

# `nix flake check` instantiates every check in one evaluator and needs
# >30 GiB, so it gets OOM-killed. flake-check evaluates them in memory-capped
# workers and builds only the uncached ones. Extra args are forwarded to the
# evaluator (e.g. --override-input). Env knobs: CHECK_FRAGMENT, WORKERS,
# MAX_MEMORY_MB, BATCH, OUT_PATHS_FILE.
check *args:
    nix run .#flake-check -- {{ args }}

update:
    nix flake update --option access-tokens "github.com=$(gh auth token)"

lint *hook:
    prek run {{ if hook == "" { "--all-files" } else { hook + " --all-files" } }}

# `nix flake check` instantiates all 3265 checks in one evaluator and needs
# >30 GiB. flake-check evaluates them in memory-capped workers instead.
check:
    nix run .#flake-check

update:
    nix flake update --option access-tokens "github.com=$(gh auth token)"

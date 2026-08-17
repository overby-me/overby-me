# Run `touch .envrc && direnv export json` to apply changes to the repository.
{
  pkgs,
  lib,
  src,
  ...
}: let
  commitlintrc = import ./commitlintrc.nix {inherit pkgs lib src;};
in {
  # Wrapped in an `if` (rather than an early `return`) because devshell module
  # shellHooks are concatenated: an early return would abort later hooks too.
  config.shellHook = ''
    # Only copy config files when running at the root of a git/jj repo.
    if [ -d .jj ] || [ -d .git ]; then
      # Copy (not symlink) configs so they are real, writable files, not symlinks into the read-only Nix store.
      install -m 644 ${./biome-nix.jsonc} biome.jsonc
      install -m 644 ${./deno.jsonc} deno.jsonc
      install -m 644 ${./lychee.toml} lychee.toml
      install -m 644 ${./rumdl.toml} rumdl.toml
      install -m 644 ${./statix.toml} statix.toml
      install -m 644 ${./typos.toml} typos.toml
      install -m 644 ${./secretsignore} .secretsignore
      install -m 644 ${commitlintrc} .commitlintrc.yml
      install -D -m 644 ${./zed/settings.jsonc} .zed/settings.json
      install -m 644 ${./ai-rules.md} .rules
      install -D -m 644 ${./ai-rules.md} .claude/rules/rules.md

      # Generate tangled workflow YAML files from Nickel config
      if [ -f .tangled/workflows.ncl ]; then
        mkdir -p .tangled/workflows
        for key in $(${pkgs.pkgsUnstable.nickel}/bin/nickel export --format yaml .tangled/workflows.ncl | ${pkgs.yq-go}/bin/yq 'keys | .[]'); do
          ${pkgs.pkgsUnstable.nickel}/bin/nickel export --format yaml .tangled/workflows.ncl \
            | ${pkgs.yq-go}/bin/yq ".$key" > ".tangled/workflows/$key.yml"
        done
      fi
    fi
  '';
}

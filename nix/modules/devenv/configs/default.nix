# Run `touch .envrc && direnv export json` to apply changes to the repository.
{
  pkgs,
  lib,
  src,
  ...
}: let
  commitlintrc = import ./commitlintrc.nix {inherit pkgs lib src;};
in {
  enterShell = ''
    # Only copy/symlink config files when running at the root of a git/jj repo
    if [ ! -d .jj ] && [ ! -d .git ]; then
      return 0 2>/dev/null || true
    fi

    ln -sf ${./biome-nix.jsonc} biome.jsonc
    ln -sf ${./deno.jsonc} deno.jsonc
    ln -sf ${./lychee.toml} lychee.toml
    ln -sf ${./rumdl.toml} rumdl.toml
    ln -sf ${./typos.toml} typos.toml
    ln -sf ${./secretsignore} .secretsignore
    ln -sf ${commitlintrc} .commitlintrc.yml
    mkdir -p .zed && cp -f ${./zed/settings.jsonc} .zed/settings.json
    cp -f ${./ai-rules.md} .rules
    mkdir -p .claude/rules && cp -f ${./ai-rules.md} .claude/rules/rules.md
    # Generate tangled workflow YAML files from Nickel config
    if [ -f .tangled/workflows.ncl ]; then
      mkdir -p .tangled/workflows
      for key in $(${pkgs.pkgsUnstable.nickel}/bin/nickel export --format yaml .tangled/workflows.ncl | ${pkgs.yq-go}/bin/yq 'keys | .[]'); do
        ${pkgs.pkgsUnstable.nickel}/bin/nickel export --format yaml .tangled/workflows.ncl \
          | ${pkgs.yq-go}/bin/yq ".$key" > ".tangled/workflows/$key.yml"
      done
    fi
  '';
}

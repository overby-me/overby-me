# Every root input must have a consumer.
#
# Flake inputs cannot belong to a project: nix resolves `inputs` from
# flake.nix before anything is evaluated, so no module, project or framework
# can add one. A project only owns its inputs where it is its own flake,
# which is what the published repos are - fe-c declares rust-overlay itself,
# and the monorepo declares nothing on its behalf.
#
# What is left for the workspace is to keep its list answerable to the tree.
# An input earns its place by being read somewhere (`inputs.<name>`) or by
# being followed by another input; anything else is a declaration nobody
# asked for. Five of them had accumulated here, one of which - gitignore -
# had been printing a warning on every evaluation because the input it was
# meant to override no longer existed.
#
# Checked by grepping the source rather than by evaluating it, because an
# input's absence cannot be observed from inside the evaluation that would
# have used it.
{
  config,
  src,
  ...
}: {
  checks.input-usage = pkgs: let
    names = builtins.attrNames config.inputs;
  in
    pkgs.runCommand "check-input-usage" {} ''
      cd ${src}
      unused=""
      for name in ${builtins.concatStringsSep " " names}; do
        # `self` is nix's own, not ours to justify.
        [ "$name" = "self" ] && continue

        read_somewhere=$(${pkgs.ripgrep}/bin/rg -l --no-messages \
          -g '!flake.nix' -g '*.nix' -g '*.nu' -g '*.yml' \
          "inputs\.$name\b" . | head -1)
        followed=$(${pkgs.ripgrep}/bin/rg --no-messages \
          "follows = \"$name\"" flake.nix | head -1)

        if [ -z "$read_somewhere" ] && [ -z "$followed" ]; then
          unused="$unused $name"
        fi
      done

      if [ -n "$unused" ]; then
        echo "root inputs that nothing reads and nothing follows:" >&2
        for name in $unused; do echo "  $name" >&2; done
        echo "Remove them, or use them where they were meant to be used." >&2
        exit 1
      fi
      touch $out
    '';
}

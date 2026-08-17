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
#
# One kind of use is invisible to a grep: the workspace takes every input
# that exports a project module, so an integration is consumed without
# `inputs.project-darwin` appearing anywhere. That is not a hole in the
# reasoning above - absence is what cannot be evaluated, and this is a
# presence - so those are recognised by the same predicate the workspace
# uses, and only the rest are grepped for.
{
  config,
  lib,
  src,
  ...
}: {
  checks.input-usage = pkgs: let
    # What flake.nix declares, read from the lock, which records exactly that
    # under root. Not `config.inputs`: a module hands its own pin to the
    # consumer by setting one, so that set holds entries nobody declared here
    # and which nothing here reads - `system-manager` arrives from the module
    # that pins it, and the file reading it lives in nix-workspace now. Those
    # are not this tree's to justify.
    declared =
      builtins.attrNames
      (builtins.fromJSON (builtins.readFile "${src}/flake.lock")).nodes.root.inputs;

    integrations =
      builtins.attrNames
      (lib.filterAttrs
        (_: i: builtins.isAttrs i && (i ? workspaceModule || i ? workspaceModules.default))
        config.inputs);
    names = lib.subtractLists integrations declared;
  in
    pkgs.runCommand "check-input-usage" {} ''
      cd ${src}
      unused=""
      for name in ${builtins.concatStringsSep " " names}; do
        # `self` is nix's own, not ours to justify.
        [ "$name" = "self" ] && continue

        # `|| true` because a search that matches nothing exits non-zero, and
        # stdenv runs this under `set -o pipefail`: without it the first input
        # that is followed rather than read - flake-compat is the one - killed
        # the whole check before reaching the `followed` test written for it.
        # Builder output is not surfaced here, so it failed with no message
        # and validated nothing.
        #
        # A declaration is `<name>.url = ...`, never `inputs.<name>`, so a
        # read inside a flake.nix counts like any other - which is how the
        # sub-flake of pins justifies itself.
        read_somewhere=$(${pkgs.ripgrep}/bin/rg -l --no-messages \
          -g '*.nix' -g '*.nu' -g '*.yml' \
          "inputs\.$name\b" . || true)
        followed=$(${pkgs.ripgrep}/bin/rg --no-messages \
          "follows = \"$name\"" -g '*.nix' . || true)

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

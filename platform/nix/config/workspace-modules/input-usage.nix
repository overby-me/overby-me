# Every root input must have a consumer.
#
# Inputs cannot belong to a project: nix resolves them from flake.nix before
# anything evaluates, so no module or framework can add one. An input earns its
# place by being read (`inputs.<name>`) or followed; five had accumulated here,
# one of them printing a warning on every evaluation because the input it
# overrode no longer existed.
#
# Grepped rather than evaluated, because an input's absence cannot be observed
# from inside the evaluation that would have used it.
{
  config,
  lib,
  src,
  ...
}: {
  checks.input-usage = pkgs: let
    # From the lock rather than `config.inputs`: a module hands its own pin to
    # the consumer by setting one, so that set holds entries nobody declared
    # here and nothing here reads.
    declared =
      lib.attrNames
      (lib.fromJSON (lib.readFile "${src}/flake.lock")).nodes.root.inputs;

    # An input exporting a module is consumed by the workspace taking it, which
    # no grep can see. Recognised by the predicate the workspace itself uses.
    integrations =
      lib.attrNames
      (lib.filterAttrs
        (_: i: lib.isAttrs i && (i ? workspaceModule || i ? workspaceModules.default))
        config.inputs);
    names = lib.subtractLists integrations declared;
  in
    pkgs.runCommand "check-input-usage" {} ''
      cd ${src}
      unused=""
      for name in ${lib.concatStringsSep " " names}; do
        # `self` is nix's own, not ours to justify.
        [ "$name" = "self" ] && continue

        # `|| true` because a search matching nothing exits non-zero under
        # stdenv's `set -o pipefail`, and builder output is not surfaced here:
        # without it the first followed-but-unread input killed the check
        # silently, before reaching the `followed` test written for it.
        #
        # A declaration is `<name>.url = ...`, never `inputs.<name>`, so a read
        # inside a flake.nix counts like any other.
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

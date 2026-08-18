# A project may only name packages inside its own namespace: every name it
# defines starts with its own short label.
#
# This is the guarantee Bazel and Buck2 provide - not derived names, but that
# you cannot claim a name outside your own package, so nobody has to
# coordinate. The module system already refuses two definitions of one name,
# but only after both exist and only saying `<unknown-file>`; this names the
# project that reached out, before the collision it would cause.
{
  options,
  config,
  lib,
  ...
}: let
  # Through `inputs` rather than a path: labels left this tree with
  # nix-workspace. Forced only inside the check below, so it cannot become the
  # `imports` loop the flake warns about.
  labels = import (config.inputs.workspace + "/labels.nix");

  exports = {
    # fe-c's runtime and cargo subcommand are published crates in their own
    # right; `fe-c-cementite` would be a name nothing else uses.
    "safety/fe-c" = ["cargo-fe-c" "cementite"];
  };

  # Four levels, because this file sits four below the root. An existence
  # check cannot catch a mistake here: one level short is `platform`, which
  # exists, so discovery quietly finds nothing and the check passes by
  # having nothing to say.
  projects = labels.discover {
    root = ../../../..;
    exclude = ["platform/nix"];
  };

  # Which project a definition came from, by the file the module system
  # recorded for it. One from outside any project - the workspace autoloading
  # packages/ by filename - is nobody's to answer for.
  projectOf = file: let
    hits = builtins.filter (l: lib.hasSuffix "/${l.path}" file || lib.hasInfix "/${l.path}/" file) projects;
  in
    if hits == []
    then null
    else builtins.head hits;

  owns = l: name:
    name == l.short || lib.hasPrefix "${l.short}-" name;

  violationsOf = d: let
    l = projectOf (toString d.file);
    allowed = exports.${l.path or ""} or [];
  in
    if l == null || builtins.isFunction d.value
    then []
    else
      map (n: "${l.label} defines ${n}, which is outside ${l.short}")
      (builtins.filter (n: !(owns l n) && !(builtins.elem n allowed))
        (builtins.attrNames d.value));

  violations =
    builtins.concatMap violationsOf
    (options.packages.definitionsWithLocations or []);
in {
  checks.namespace-ownership = pkgs:
    if violations != []
    then
      throw ''
        packages named outside their project's namespace:
          ${builtins.concatStringsSep "\n  " violations}
        Rename them into the project's namespace, or add the name to `exports`
        in platform/nix/config/workspace-modules/namespaces.nix if it is published under
        an identity of its own.
      ''
    else "${pkgs.coreutils}/bin/true";
}

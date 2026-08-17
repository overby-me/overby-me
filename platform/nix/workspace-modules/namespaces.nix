# A project may only name packages inside its own namespace.
#
# This is the guarantee Bazel and Buck2 actually provide. Not derived names -
# a BUILD file does type `name = "lib"` - but that you cannot claim a name
# outside your own package, so `//foo:lib` and `//bar:lib` never race and
# nobody has to coordinate. Here the namespace is the project's label, and a
# flat flake attribute is a rendering of it, so the rule is that every name a
# project defines starts with its own short label.
#
# The module system already refuses two definitions of one name, but only
# after both exist and only saying `<unknown-file>`. This says which project
# reached outside its namespace, before the collision it would eventually
# cause.
#
# `exports` is the deliberate exception, and it is spelled out rather than
# inferred: a crate that is published under its own identity keeps that
# identity here, the same way a package wrapping upstream software keeps the
# upstream name.
{
  options,
  lib,
  ...
}: let
  labels = import ../project/labels.nix;

  exports = {
    # fe-c's runtime and cargo subcommand are published crates in their own
    # right; `fe-c-cementite` would be a name nothing else uses.
    "safety/fe-c" = ["cargo-fe-c" "cementite"];
  };

  projects = labels.discover {
    root = ../../..;
    exclude = ["platform/nix"];
  };

  # Which project a definition came from, by the file the module system
  # recorded for it. A definition from outside any project - flakelight's own
  # nixDir autoloading, which names platform/nix/packages by filename - is not a
  # project's to answer for.
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
        in platform/nix/workspace-modules/namespaces.nix if it is published under
        an identity of its own.
      ''
    else "${pkgs.coreutils}/bin/true";
}

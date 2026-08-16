# Identity for a monorepo: a label, not a name.
#
# Bazel, Buck2 and Piper all answer "how do two teams avoid naming the same
# thing" the same way, and none of the answers is a naming convention:
#
#   Bazel   @repo//package/path:target
#   Buck2   cell//package:target
#   Piper   //depot/google3/path, one version of everything, at head
#
# In all three, identity is (namespace root, path, local name), and the path
# comes from the filesystem. A target name only has to be unique inside its
# own directory, so two teams can both ship `:lib` and never coordinate.
# Global uniqueness is structural: a property of the tree rather than of
# anyone's discipline. That is why it survives at Piper's scale, where a
# naming convention would need every engineer to keep holding it.
#
# A Nix flake has none of that. `packages.<system>.<name>` is one flat
# namespace over the whole repo, so a monorepo hand-prefixes to survive:
# eighty-one of this tree's packages are called `rust-something`, and that
# prefix is doing by convention the job a label does by construction.
#
# So the label is the identity, and every flat name is a *rendering* of one.
# Renderings can collide; labels cannot. When two collide, this reports both
# labels rather than picking one, which is the part a convention can never
# do: a flat namespace cannot tell you what it just shadowed.
#
# ## Vocabulary
#
# This tree's own words, not a build system's:
#
#   area      a namespace root that someone owns   (Bazel repo, Buck2 cell)
#   project   the addressable unit                 (Bazel/Buck2 package)
#   target    an output inside a project
#
# ## Neutrality
#
# Depends on nothing. josh and nix are the constants here; the build system
# is a slot the README fills with Buck2 today and may fill with vixen or
# something else tomorrow. So the projections that live in this file are the
# two universal ones, and anything build-system-shaped is an adapter in
# ./adapters, written against the label rather than the other way round.
#
#   nix     packages.<system>.safety-oxidized-sed, plus the short alias
#   josh    :/safety/oxidized/sed
let
  inherit (builtins) attrNames concatMap concatStringsSep elem elemAt filter head isAttrs isString length listToAttrs pathExists readDir substring tail;

  segsOf = p: filter (x: x != "" && x != ".") (filter isString (builtins.split "/" p));

  # The areas this tree is divided into. A path outside all of them belongs
  # to no area and keeps a bare label, which is how a project at the top
  # level stays addressable without being given an owner it does not have.
  defaultAreas = ["ai" "apps" "dev" "media" "platform" "safety"];
in rec {
  # Every directory holding `marker` is a project, in Bazel's sense of the
  # directory a BUILD file makes addressable. The walk stops there: what is
  # inside a project is that project's business, including markers of its
  # own.
  discover = {
    root,
    areas ? defaultAreas,
    exclude ? [],
    # A directory is a project if it holds any of these. Several, so a tree
    # can migrate one project at a time rather than in one commit.
    markers ? ["default.nix"],
    depth ? 4,
  }: let
    walk = rel: remaining: let
      dir =
        if rel == ""
        then root
        else root + "/${rel}";
      under = name:
        if rel == ""
        then name
        else "${rel}/${name}";
      children =
        filter
        (name: (readDir dir).${name} == "directory" && substring 0 1 name != ".")
        (attrNames (readDir dir));
    in
      if elem rel exclude
      then []
      else if rel != "" && builtins.any (m: pathExists (dir + "/${m}")) markers
      then [rel]
      else if remaining == 0
      then []
      else concatMap (name: walk (under name) (remaining - 1)) children;
  in
    map (path: labelOf {inherit path areas;}) (walk "" depth);

  # A path becomes a label: the first segment is the area when it names one,
  # the rest is the project, and the target defaults to the project's own
  # basename. That default is Bazel's `//foo/bar` meaning `//foo/bar:bar`,
  # and it is what keeps the common case free of a name to invent.
  labelOf = {
    path,
    areas ? defaultAreas,
    target ? null,
  }: let
    segs = segsOf path;
    inArea = length segs > 1 && elem (head segs) areas;
    area =
      if inArea
      then head segs
      else "";
    projectSegs =
      if inArea
      then tail segs
      else segs;
    basename = elemAt projectSegs (length projectSegs - 1);
    name =
      if target != null
      then target
      else basename;
    # A target that is the project's own name adds nothing to a rendering,
    # exactly as `//foo/bar:bar` is written `//foo/bar`.
    suffix =
      if name == basename
      then []
      else [name];
    shortName = concatStringsSep "-" (projectSegs ++ suffix);
  in {
    inherit area path;
    # The package part of the label, in Bazel's sense: the path within the
    # area. Not called `project`, because the value this belongs to *is* the
    # project and `project.project` says nothing.
    pkg = concatStringsSep "/" projectSegs;
    target = name;

    # The form to print in an error: the only one that is always
    # unambiguous. Shared syntax with Bazel and Buck2 because it is the
    # lingua franca for this, not because either is depended on.
    label = "${area}//${concatStringsSep "/" projectSegs}:${name}";

    # The flat name a flake can hold. Total and injective over labels: the
    # area leads, and no component may contain a slash, so joining on "-"
    # cannot alias two different labels onto one string.
    flat = concatStringsSep "-" (
      (
        if area == ""
        then []
        else [area]
      )
      ++ projectSegs
      ++ suffix
    );

    # The name to type. Bazel calls this the apparent name: short, and valid
    # only where nothing else claims it. `render` emits it when it is
    # unique and withholds it when it is not.
    short = shortName;

    # The subtree that publishes this project as a repo of its own. josh
    # addresses a subtree the way a label addresses a project, so this is a
    # rendering rather than a translation. It is the *current* location
    # only; see `filterOf` for why that is not the whole filter.
    josh = ":/${path}";

    # Qualify what this project calls a target, into what the flake calls it.
    # `default` is the project itself, which is Bazel's `//foo/bar` meaning
    # `//foo/bar:bar`; anything else hangs off it.
    #
    #   qualify "dev"                     -> "oxidized-awk-dev"
    #   qualify { default = a; dev = b; } -> { oxidized-awk = a;
    #                                          oxidized-awk-dev = b; }
    #
    # One name for both, because it is one idea: a file writes the target
    # names it uses locally, and cannot write one that lands in another
    # project's namespace. Taking the set as well as the string is what makes
    # that convenient enough to actually do - a whole `packages` or `checks`
    # block goes through it at once.
    qualify = arg: let
      one = target:
        if target == "default" || target == ""
        then shortName
        else "${shortName}-${target}";
    in
      if isAttrs arg
      then
        listToAttrs (map (target: {
            name = one target;
            value = arg.${target};
          })
          (attrNames arg))
      else one arg;
  };

  # Renderings, with the ambiguous ones separated rather than merged. What a
  # flat namespace cannot tell you is *which two* things collided. A label
  # can, so that is what the report says.
  render = labels: let
    claimedBy = short: filter (l: l.short == short) labels;
    ambiguous = filter (l: length (claimedBy l.short) > 1) labels;
    unique = filter (l: length (claimedBy l.short) == 1) labels;
  in {
    inherit labels;
    # One per label, always available.
    canonical = listToAttrs (map (l: {
        name = l.flat;
        value = l;
      })
      labels);
    # Only where unambiguous.
    aliases = listToAttrs (map (l: {
        name = l.short;
        value = l;
      })
      unique);
    conflicts = map (l: {inherit (l) short label;}) ambiguous;
    report =
      if ambiguous == []
      then null
      else
        "these labels render to the same short name: "
        + concatStringsSep "; " (map (l: "${l.label} -> ${l.short}") ambiguous)
        + ". Both keep their canonical name; move one or address them by it.";
  };

  # A published repo's filter is not a function of where a project is. It is
  # a function of everywhere it has *been*: josh reconstructs history, so a
  # filter that names only today's path publishes a repo whose history
  # starts at the last move. Labels are spatial and filters are
  # spatiotemporal, which is the one place this vocabulary does not reach on
  # its own, and the reason `formerPaths` has to be recorded rather than
  # derived.
  #
  # Bazel and Buck2 never meet this: they address a tree, they do not
  # federate one.
  # `eras` is every location the project has occupied, oldest first, each
  # with the sibling directories it depended on at the time: a dependency
  # moves house too, so the pairing has to be per era rather than global.
  #
  # `kind` is what the project is made of, and there are two:
  #
  #   crate  its .nix files are the monorepo's build of it, which the
  #          published repo replaces with a generated flake, so they are
  #          excluded and only flake.nix is let back through
  #   nix    its .nix files ARE the project, so nothing is excluded
  #
  # Those two rules reproduce all thirty-nine filters this tree publishes,
  # byte for byte.
  filterOf = {
    eras,
    kind ? "crate",
  }: let
    baseOf = p: let
      s = segsOf p;
    in
      elemAt s (length s - 1);

    subtreeTerms = era: let
      p = era.path;
      deps = era.deps or [];
    in
      if kind == "nix"
      then [":/${p}"]
      else if deps == []
      then [":/${p}:exclude[::*.nix]" ":/${p}::flake.nix"]
      else
        # The project and each dependency keep their own directory name, so
        # that a `path = "../pcre2"` in a Cargo.toml still resolves once
        # they are side by side in a repo of their own. The project's own
        # generated files are then lifted back to the repo root, where a
        # visitor lands.
        [
          ":/${p}:exclude[::*.nix]:exclude[::flake.lock]:exclude[::.tangled/]:exclude[::README.md]:prefix=${baseOf p}"
        ]
        ++ map (d: ":/${d}:exclude[::*.nix]:prefix=${baseOf d}") deps
        ++ [
          ":/${p}::flake.nix"
          ":/${p}::flake.lock"
          ":/${p}::README.md"
          ":/${p}::.tangled/"
        ];
  in ":[${concatStringsSep "," (concatMap subtreeTerms eras)}]:unsign";
}

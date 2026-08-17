# A directory tree of outputs, in this tree's own vocabulary.
#
# This is what flakelight's nixDir does, written here so the whole convention
# belongs to one place: a directory names the output it feeds, a file names
# the entry in it, and nothing inside either says what it is called. The path
# is the address, the same rule projects follow.
#
#   platform/nix/packages/datui.nix        -> packages.datui
#   platform/nix/nixos-modules/core/       -> nixosModules.core
#   platform/nix/with-overlays/rust.nix    -> an entry of withOverlays
#
# Directory names are kebab-case, because every other directory here is, and
# the option name they feed is derived from them rather than listed: the
# transform runs from the option name to the directory, which is total,
# instead of from a human name to an option, which needs a dictionary of
# exceptions (nixos/config would have to mean nixosConfigurations while
# home-manager/config means homeConfigurations).
dir: {
  options,
  config,
  lib,
  ...
}: let
  inherit (builtins) attrNames elem filter pathExists readDir;
  inherit (lib) attrValues concatStrings genAttrs hasSuffix mkMerge removePrefix removeSuffix stringToCharacters subtractLists toLower toUpper;

  kebab = name:
    concatStrings (map (c:
      if c == toUpper c && c != toLower c
      then "-" + toLower c
      else c) (stringToCharacters name));

  # An entry is a .nix file, or a directory holding default.nix. A leading
  # underscore is dropped from the name, which is how a file orders itself in
  # a directory listing without that showing up in the flake.
  entriesIn = path: let
    names =
      map (removePrefix "_")
      (map (removeSuffix ".nix")
        (filter (
            s:
              s
              != "default.nix"
              && (hasSuffix ".nix" s || pathExists (path + "/${s}/default.nix"))
          )
          (attrNames (readDir path))));
  in
    genAttrs names (p:
      if pathExists (path + "/_${p}.nix")
      then path + "/_${p}.nix"
      else if pathExists (path + "/${p}.nix")
      then path + "/${p}.nix"
      else path + "/${p}");

  # An option is fed by `<dir>/<kebab name>`, as a single value when that is
  # a file or holds a default.nix, and entry by entry otherwise. Whether the
  # entries arrive as a set or as a list is decided by the option's own type,
  # because a list-valued output like withOverlays wants the values and an
  # attribute-valued one wants them keyed.
  # An option set - `nixpkgs`, which only exists because `nixpkgs.config`
  # does - is not an output and has no type to ask about.
  isOption = name: options.${name} ? type;

  # Whether an option ultimately holds modules, found by walking its type
  # rather than by naming the option. `lazyAttrsOf module` is wrapped in a
  # coercedTo by optCallWith, so the module type is never the outer one.
  holdsModules = type:
    type.name
    == "module"
    || builtins.any holdsModules (attrValues (type.nestedTypes or {}));

  valueFor = name: let
    path = dir + "/${kebab name}";
    type = options.${name}.type;
    # A module keeps the path it came from. Import it and what lands in the
    # output is an anonymous function, which the module system cannot tell
    # apart from any other: the same module reaching evalModules twice then
    # declares its options twice and evaluation fails. A path has identity,
    # and names the file in an error besides.
    #
    # Derived from the type rather than listed, because listing it got two of
    # the three module collections and missed nixosModules, which has exactly
    # the type darwinModules has. Nothing had broken yet only because nothing
    # imported one of them by two routes.
    asPaths = holdsModules type || elem name (config.nixDirPathAttrs or []);
  in
    if pathExists (dir + "/${kebab name}.nix")
    then [(import (dir + "/${kebab name}.nix"))]
    else if pathExists (path + "/default.nix")
    then [(import path)]
    else if pathExists path
    then let
      paths = entriesIn path;
      asAttrs =
        if asPaths
        then paths
        else builtins.mapAttrs (_: import) paths;
      asList = attrValues asAttrs;
    in
      if type.check asAttrs
      then [asAttrs]
      else if type.check asList
      then [asList]
      else [asAttrs]
    else [];

  # Everything the flake can output, minus what describes the mechanism.
  outputNames =
    builtins.filter isOption
    (subtractLists ["_module" "nixDir" "nixDirAliases" "nixDirPathAttrs"]
      (attrNames options));
in {
  # nixDirPathAttrs is read here but declared in mk-flake.nix, because a
  # module may set it whether or not anyone scans a directory. Declaring it
  # here made it exist only for a consumer that had set `outputDirs`, so a
  # module setting it failed with "the option does not exist" for everyone
  # else. This tree always sets outputDirs, which is why nothing here saw it.
  config = genAttrs outputNames (name: mkMerge (valueFor name));
}

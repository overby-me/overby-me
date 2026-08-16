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
  valueFor = name: let
    path = dir + "/${kebab name}";
    asPaths = elem name (config.nixDirPathAttrs or []);
    type = options.${name}.type;
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
    subtractLists ["_module" "nixDir" "nixDirAliases" "nixDirPathAttrs"]
    (attrNames options);
in {
  # Which options want the path to an entry rather than the imported value.
  # Declared here because this is what reads it: a directory of NixOS modules
  # is imported by the configuration that uses it, not by us.
  options.nixDirPathAttrs = lib.mkOption {
    type = lib.types.listOf lib.types.str;
    default = [];
  };

  config = genAttrs outputNames (name: mkMerge (valueFor name));
}

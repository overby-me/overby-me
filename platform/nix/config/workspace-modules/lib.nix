# Auto-discovers lib functions from platform/nix/config/lib/.
# Each .nix file can be an attrset or a function taking lib, returning an attrset.
# Results are merged flat into the lib flake output.
# Uses mkForce to override nixDir's raw auto-discovery with resolved values.
#
# Discovery rules:
#   - .nix files are imported, resolved (called with lib if a function), and merged.
#   - Directories with default.nix are imported as a single unit.
#   - Directories without default.nix are recursed into.
#   - Files and directories starting with _ are considered private and skipped.
#
# Modules that define `perSystemLib` attrs are automatically routed to
# the perSystemLib option (system-dependent functions taking pkgs).
# All other attrs are merged into the flake's lib output (system-independent).
{lib, ...}: let
  # Named directly rather than through nixDir: this is a second rule over the
  # same directory - resolve each file and merge the results flat - and it
  # runs whether or not anything else is scanning the tree.
  libDir = ../lib;
  hasLibDir = lib.pathExists libDir;

  resolve = v:
    if lib.isFunction v
    then v lib
    else v;

  # Recursively discover and import .nix files from a directory.
  importLibDir = dir: let
    entries = lib.readDir dir;
    names = lib.attrNames entries;

    processEntry = name: let
      type = entries.${name};
      path = dir + "/${name}";
    in
      if lib.hasPrefix "_" name
      then {}
      else if type == "regular" && lib.hasSuffix ".nix" name
      then resolve (import path)
      else if type == "directory"
      then
        if lib.pathExists (path + "/default.nix")
        then resolve (import path)
        else importLibDir path
      else {};
  in
    # Merge perSystemLib one level deep: several lib directories each
    # contribute perSystemLib entries, and a shallow // would keep only the
    # alphabetically last directory's set.
    lib.foldl' (
      acc: name: let
        e = processEntry name;
      in
        acc
        // e
        // lib.optionalAttrs (acc ? perSystemLib || e ? perSystemLib) {
          perSystemLib = (acc.perSystemLib or {}) // (e.perSystemLib or {});
        }
    ) {}
    names;

  discovered =
    if hasLibDir
    then importLibDir libDir
    else {};

  # Separate perSystemLib attrs from pure lib attrs.
  mergedPerSystemLib = discovered.perSystemLib or {};
  mergedLib = removeAttrs discovered ["perSystemLib"];
  # A tool's checks live inside that tool's library, next to what they test,
  # rather than in the directory of flake modules. This walk already knows
  # every library, so it imports them too instead of the root flake naming
  # each one - which is how buck2's came to be listed after skylark's and
  # nobody noticed the two had swapped.
  #
  # `imports` may not depend on `config`, and these depend only on paths.
  checkModules = let
    entries = lib.readDir libDir;
  in
    map (name: libDir + "/${name}/checks.nix")
    (lib.filter (
        name:
          entries.${name}
          == "directory"
          && lib.pathExists (libDir + "/${name}/checks.nix")
      )
      (lib.attrNames entries));
in {
  imports = checkModules;

  lib = lib.mkForce mergedLib;
  perSystemLib = mergedPerSystemLib;
}

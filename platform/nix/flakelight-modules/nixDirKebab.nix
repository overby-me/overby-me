# Let a nixDir directory be named the way directories are named here.
#
# flakelight scans a directory named exactly after the output option, which is
# camelCase: nixosModules, homeConfigurations, withOverlays. Every other
# directory in this tree is lowercase with hyphens, so those fourteen stood
# out as the only camelCase paths in the repository.
#
# nixDirAliases exists to accept a second name per option, and it was fourteen
# hand-written lines until they were renamed away. This computes it instead:
# every option's name, rendered in kebab-case, is accepted as its directory.
#
# Deducing an option name from a *human* directory name is what does not work
# - nixos/config would have to mean nixosConfigurations and home-manager/config
# homeConfigurations, which needs a dictionary of exceptions. Going the other
# way is a total function, because it starts from the name that already exists
# rather than guessing at it. Nothing has to be listed, and an output added to
# flakelight tomorrow gets its directory for free.
{
  options,
  lib,
  ...
}: let
  inherit (lib) concatStrings stringToCharacters toLower toUpper;

  kebab = name:
    concatStrings (map (c:
      if c == toUpper c && c != toLower c
      then "-" + toLower c
      else c) (stringToCharacters name));

  # `_module` and nixDir's own settings are not outputs.
  outputNames =
    builtins.filter (n: n != "_module" && n != "nixDir" && n != "nixDirAliases" && n != "nixDirPathAttrs")
    (builtins.attrNames options);
in {
  nixDirAliases =
    builtins.listToAttrs
    (map (n: {
        name = n;
        value = [(kebab n)];
      })
      (builtins.filter (n: kebab n != n) outputNames));
}

# The other half of nix-project. `project` is one published unit, and builds
# a whole flake for it; `workspace` is the tree those units live in, and finds
# them.
#
# A workspace closes over nothing, unlike `project`. A published repo wants
# our nixpkgs, so `project` supplies it; a monorepo pins nixpkgs and thirty
# other flakes for reasons of its own, and its flakelight has to be the one
# its other modules were written against. So this is a plain function, and
# takes both from the caller's inputs.
#
#   outputs = inputs:
#     workspace ./. {
#       inherit inputs;
#       systems = [...];
#       nixDir = ./platform/nix;
#       projects.exclude = ["platform/nix"];
#     };
#
# Everything except `projects` is passed to flakelight untouched.
root: cfg: let
  inherit (builtins) attrNames concatMap elem filter pathExists readDir stringLength substring;

  discovery = cfg.projects or {};
  exclude = discovery.exclude or [];
  # Deep enough for area/group/project (safety/oxidized/awk) with one to
  # spare. Bounded rather than unbounded because the cost of an extra level
  # is a readDir of everything at that level, on every evaluation.
  depth = discovery.depth or 4;

  # A project is a directory holding a default.nix. The walk stops there
  # rather than descending, so a project's own subdirectories are its
  # business: several of them carry a default.nix of their own that is
  # imported by the project, not by the workspace.
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
    else if rel != "" && pathExists (dir + "/default.nix")
    then [dir]
    else if remaining == 0
    then []
    else concatMap (name: walk (under name) (remaining - 1)) children;

  discovered = walk "" depth;

  # A module directory is imported file by file: every .nix in it is a
  # flakelight module. One level only, and no default.nix rule, because
  # these are not projects: naming the directory is the whole point, and a
  # subdirectory of it would be a library the modules share rather than
  # another module.
  isNixFile = name: substring (stringLength name - 4) 4 name == ".nix";

  modulesIn = dir:
    map (name: dir + "/${name}")
    (filter
      (name:
        (readDir dir).${name}
        == "regular"
        && substring 0 1 name != "."
        && isNixFile name)
      (attrNames (readDir dir)));

  fromModuleDirs = concatMap modulesIn (cfg.moduleDirs or []);
in
  cfg.inputs.flakelight root (
    (removeAttrs cfg ["projects" "moduleDirs"])
    // {
      # Explicit first, then whole directories, then projects. A workspace
      # can still name one module: the four lib checks are a file inside a
      # library rather than a directory of modules.
      imports = (cfg.imports or []) ++ fromModuleDirs ++ discovered;
    }
  )

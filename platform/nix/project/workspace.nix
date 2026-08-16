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
  inherit (builtins) attrNames concatMap filter readDir stringLength substring throw;

  labels = import ./labels.nix;

  discovery = cfg.projects or {};

  # One walk, shared with the identity layer, so a project is the same thing
  # to both: the directory a `default.nix` makes addressable. Duplicating it
  # here is how the two would come to disagree about what a project is.
  found = labels.discover ({
      inherit root;
    }
    // discovery);

  # A label per project, so a collision is reported as the two labels that
  # produced it rather than as whichever definition the module system saw
  # last. This is the point of having labels at all: a flat namespace cannot
  # tell you what it just shadowed.
  named = labels.render found;

  discovered =
    if named.report != null
    then throw "workspace: ${named.report}"
    else map (l: root + "/${l.path}") found;

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

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
  inherit (builtins) attrNames concatMap filter pathExists readDir stringLength substring throw;

  labels = import ./labels.nix;

  discovery = cfg.projects or {};

  # One walk, shared with the identity layer, so a project is the same thing
  # to both: the directory a `default.nix` makes addressable. Duplicating it
  # here is how the two would come to disagree about what a project is.
  found = labels.discover ({
      inherit root;
      markers = ["project.nix" "default.nix"];
    }
    // discovery);

  # A label per project, so a collision is reported as the two labels that
  # produced it rather than as whichever definition the module system saw
  # last. This is the point of having labels at all: a flat namespace cannot
  # tell you what it just shadowed.
  named = labels.render found;

  # A `project.nix` is a module applied to its own label, which is what lets
  # a file state what it builds without stating where the names come from:
  #
  #   label: {...}: { packages = label.names { default = ...; dev = ...; }; }
  #
  # The module system cannot hand one module a different argument from
  # another - `_module.args` is evaluation-wide - so the label is applied at
  # import rather than passed through it. That is only unambiguous because
  # every file found this way is the same kind of thing, which is the
  # dendritic part: one uniform rule, and the tree decides the names.
  #
  # A `default.nix` is imported as an ordinary flakelight module, so a
  # project migrates when there is a reason to and not before.
  moduleOf = l:
    if pathExists (root + "/${l.path}/project.nix")
    then import (root + "/${l.path}/project.nix") l
    else root + "/${l.path}";

  discovered =
    if named.report != null
    then throw "workspace: ${named.report}"
    else map moduleOf found;

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

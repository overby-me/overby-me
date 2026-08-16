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
  inherit (builtins) attrNames concatMap elem filter isAttrs isFunction mapAttrs pathExists readDir stringLength substring throw;

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

  # A `project.nix` is an ordinary flakelight module. What makes it a project
  # module is that the workspace names its outputs, so the file declares what
  # it builds using local names and never says where they land:
  #
  #   {lib, ...}: {
  #     packages = { default = ...; dev = ...; };
  #     checks   = { boot = ...; };
  #   }
  #
  # There is no call to wrap it in, because the workspace already has the
  # file and the identity: it applies one to the other. A module that wants
  # to know where it is takes `project` among its arguments, which works
  # because the workspace applies the function itself rather than handing it
  # to the module system - `_module.args` is evaluation-wide and could not
  # give one module a different value from another.
  #
  # `imports`, `options` and `config` are structure rather than names, so
  # they pass through. Everything else that is a set of names is qualified,
  # and a name beginning with `/` opts out.
  structural = ["imports" "options" "config" "_module"];

  qualifyOutputs = l:
    mapAttrs (
      name: value:
        if elem name structural || !(isAttrs value)
        then value
        else l.qualify value
    );

  moduleOf = l: let
    file = root + "/${l.path}/project.nix";
    module = import file;
  in
    if !(pathExists file)
    then root + "/${l.path}"
    else if isFunction module
    then (args: qualifyOutputs l (module (args // {project = l;})))
    else qualifyOutputs l module;

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

  # A tree of outputs, replacing flakelight's nixDir with the same rule
  # projects follow: the path is the address. See ./outputs.nix.
  fromOutputDirs = map (d: import ./outputs.nix d) (cfg.outputDirs or []);
in
  cfg.inputs.flakelight root (
    (removeAttrs cfg ["projects" "moduleDirs" "outputDirs"])
    // {
      # Explicit first, then whole directories, then projects. A workspace
      # can still name one module: the four lib checks are a file inside a
      # library rather than a directory of modules.
      imports = (cfg.imports or []) ++ fromOutputDirs ++ fromModuleDirs ++ discovered;
    }
  )

# What this tree's nix configuration talks to.
#
# The modules taken out of nix-workspace, declared here rather than in the
# root flake. The framework itself is not among them: the root calls it, and
# nothing in this directory talks to it.
#
# Each module is a flake owning one upstream and the module that uses it, so a
# tree takes only what it talks to and the name says what taking it costs -
# and this is the directory doing the talking.
#
# It only carries inputs. A flake's own source is its directory, so this one
# cannot see safety/ or apps/, and discovery over the whole tree stays with
# the root flake, which reaches these through `inputs` on this input.
#
# nixpkgs comes from the root rather than being pinned twice: without that a
# module builds its upstream against a nixpkgs of its own, which is how
# zen-browser came to want an ffmpeg the rest of the tree does not have.
{
  description = "The framework and the modules this tree's nix configuration uses";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

    # Read by the overlays in this directory, and by nothing above it.
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # For `outputsIn`, which turns this directory into a module. The consuming
    # flake follows its own copy onto this, so there is one framework.
    workspace = {
      url = "git+https://tangled.org/overby.me/nix-workspace";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };

    workspace-darwin = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/darwin";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-disko = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/disko";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-drowse = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/drowse";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-home-manager = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-modular-skills = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/modular-skills";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-nixos-hardware = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/nixos-hardware";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-nixos-raspberrypi = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/nixos-raspberrypi";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-nix-wallpaper = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/nix-wallpaper";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-ragenix = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/ragenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-system-manager = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/system-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-zen-browser = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/zen-browser";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Real Cargo workspaces, which the cargo library is tested against and the
    # ninja library builds its graph extractor from. They were sibling
    # directories reached by `../../../..`, which is a thing this flake cannot
    # see: a flake's source is its own directory, so those paths worked only
    # because the monorepo happened to be what surrounded it, and they broke
    # the moment it moved. As inputs they are the same source either way, and
    # the tree above overrides them onto its own copies so a change to a port
    # is still what gets tested.
    #
    # Fixtures rather than vendored copies on purpose: what these catch is the
    # cargo library meeting a manifest feature it does not handle, which a
    # snapshot stops doing the day after it is taken.
    #
    # Each follows onto the framework above rather than fetching its own, so
    # a tree that takes this has one nix-workspace rather than four.
    wclip = {
      url = "git+https://tangled.org/overby.me/wclip";
      inputs.workspace.follows = "workspace";
    };
    oxidized-xz = {
      url = "git+https://tangled.org/overby.me/oxidized-xz";
      inputs.workspace.follows = "workspace";
    };
    oxidized-ninja = {
      url = "git+https://tangled.org/overby.me/oxidized-ninja";
      inputs.workspace.follows = "workspace";
    };
  };

  # A module, so this directory is a workspace rather than a bag of inputs:
  # it carries the outputs under it and the modules it takes, and a tree that
  # has this input gets both by having it.
  #
  # A module rather than outputs of its own, because outputs would be a second
  # evaluation, and the thirty-four projects elsewhere in the tree build with
  # a `lib` defined in here - a lib in another flake's evaluation is not one
  # they can reach.
  #
  # A plain attribute set, not `{lib, ...}: {...}`. `imports` is read before
  # module arguments exist, so building it out of one is the loop the module
  # system warns about as referencing `config` in `imports`: it surfaces as
  # infinite recursion inside whichever module happens to want `pkgs` first.
  # Everything here comes from this flake's own closure instead.
  outputs = inputs: let
    inherit (inputs.nixpkgs) lib;
    modules = lib.filterAttrs (n: _: lib.hasPrefix "workspace-" n) inputs;

    # The modules of this tree, which used to be found by the consuming flake
    # because it named this directory. It carries them itself now, so having
    # this input is the whole of having them.
    own =
      map (n: ./workspace-modules + "/${n}")
      (builtins.attrNames (builtins.readDir ./workspace-modules));
  in {
    workspaceModule = {
      imports =
        [(inputs.workspace.outputsIn ./.)]
        ++ own
        ++ (lib.mapAttrsToList (_: i: i.workspaceModule) modules);

      # What this directory pins, handed to the consuming tree the way the
      # modules beside it hand over theirs: the overlays in with-overlays/
      # reach rust-overlay and nixpkgs-unstable through `pkgs.inputs`, which
      # is this set.
      # mkDefault, like the modules beside it: the consuming tree names some of
      # these too - nixpkgs at least - and `inputs` takes one definition per
      # name, so handing them over plainly is a collision rather than a merge.
      inputs = lib.mapAttrs (_: lib.mkDefault) (removeAttrs inputs ["self"]);

      # Where this directory is, for anything deriving a path from it - the
      # secrets module takes <outputDir>/secrets from here.
      outputDirs = [./.];
    };
  };
}

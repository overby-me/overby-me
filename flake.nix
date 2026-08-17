{
  description = "Monorepo";

  nixConfig = {
    extra-substituters = ["https://overby-me.cachix.org"];
    extra-trusted-public-keys = ["overby-me.cachix.org-1:dU7qOj5u97QZz98nqnh+Nwait6c+2d2Eq0KTOAXTyp4="];
  };

  inputs = {
    # Nix
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Development
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Transitive flake dependencies
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-compat.follows = "flake-compat";
      };
    };
    flake-compat = {
      url = "github:edolstra/flake-compat";
    };
  };

  # nix-workspace finds the projects in this tree, so this file does not list
  # them. It used to live here and be published from here, which is why it was
  # imported by path: taking it as an input would have meant publishing a
  # change to it before this tree could evaluate that change. It is developed
  # in its own repo now, so it is an ordinary input, and this tree consumes it
  # exactly the way anyone else does - which is the point, because the two
  # bugs it shipped were both ones only an input-consumer could hit.
  #
  # Its modules are selected one at a time out of the same repo. Each is a
  # flake owning one upstream and the module that uses it, so a tree takes
  # only what it talks to and its name says what taking it costs.
  #
  # Each follows this tree's nixpkgs. Without that a module builds its
  # upstream against a nixpkgs of its own, which is how zen-browser came to
  # want an ffmpeg the rest of the tree does not have.
  inputs = {
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
  };

  # The projects this tree publishes, taken as inputs so it builds the flakes
  # it ships and holds them against its own build of the same source. A flake
  # nothing evaluates is the problem these had in the first place, and `?dir=`
  # makes one checkable by hand but nothing runs it. See project-flakes.nix
  # and platform/tangled/publish/README.md.
  #
  # Each is a path to the project's own flake, which names nix-workspace by
  # the same URL a clone does - the framework is its own repo, so there is no
  # longer an in-tree spelling and a published spelling to keep in step.
  #
  # Each follows this tree's copy of the framework rather than resolving its
  # own, so all twenty-two build against the one nix-workspace declared above,
  # already pointed at this tree's nixpkgs and git-hooks. Overriding those two
  # per project instead would say the same thing twenty-two times and leave a
  # separate node per occurrence: nix locks one even when every one resolves
  # to the same revision, which is how an earlier arrangement put 25
  # flake-compat and 22 git-hooks nodes in this lock at a single rev. A clone
  # has no such override and gets what nix-workspace pins.
  #
  # Not every project: the list is the cheap end, and grows deliberately
  # because each one costs a second build of the crate. The two projects with
  # sibling dependencies are absent for a different reason - they publish as
  # several directories with the crate one level down, a shape their own
  # directory does not have.
  inputs = {
    oxidized-awk = {
      url = "path:./safety/oxidized/awk";
      inputs.workspace.follows = "workspace";
    };
    oxidized-bash = {
      url = "path:./safety/oxidized/bash";
      inputs.workspace.follows = "workspace";
    };
    oxidized-binutils = {
      url = "path:./safety/oxidized/binutils";
      inputs.workspace.follows = "workspace";
    };
    oxidized-bison = {
      url = "path:./safety/oxidized/bison";
      inputs.workspace.follows = "workspace";
    };
    oxidized-bubblewrap = {
      url = "path:./safety/oxidized/bubblewrap";
      inputs.workspace.follows = "workspace";
    };
    oxidized-bzip2 = {
      url = "path:./safety/oxidized/bzip2";
      inputs.workspace.follows = "workspace";
    };
    oxidized-diffutils = {
      url = "path:./safety/oxidized/diffutils";
      inputs.workspace.follows = "workspace";
    };
    oxidized-file = {
      url = "path:./safety/oxidized/file";
      inputs.workspace.follows = "workspace";
    };
    oxidized-gcc = {
      url = "path:./safety/oxidized/gcc";
      inputs.workspace.follows = "workspace";
    };
    oxidized-gzip = {
      url = "path:./safety/oxidized/gzip";
      inputs.workspace.follows = "workspace";
    };
    oxidized-help2man = {
      url = "path:./safety/oxidized/help2man";
      inputs.workspace.follows = "workspace";
    };
    oxidized-llvm = {
      url = "path:./safety/oxidized/llvm";
      inputs.workspace.follows = "workspace";
    };
    oxidized-make = {
      url = "path:./safety/oxidized/make";
      inputs.workspace.follows = "workspace";
    };
    oxidized-ninja = {
      url = "path:./safety/oxidized/ninja";
      inputs.workspace.follows = "workspace";
    };
    oxidized-perl = {
      url = "path:./safety/oxidized/perl";
      inputs.workspace.follows = "workspace";
    };
    oxidized-pipewire = {
      url = "path:./safety/oxidized/pipewire";
      inputs.workspace.follows = "workspace";
    };
    oxidized-patch = {
      url = "path:./safety/oxidized/patch";
      inputs.workspace.follows = "workspace";
    };
    oxidized-patchelf = {
      url = "path:./safety/oxidized/patchelf";
      inputs.workspace.follows = "workspace";
    };
    oxidized-pcre2 = {
      url = "path:./safety/oxidized/pcre2";
      inputs.workspace.follows = "workspace";
    };
    oxidized-sed = {
      url = "path:./safety/oxidized/sed";
      inputs.workspace.follows = "workspace";
    };
    oxidized-texinfo = {
      url = "path:./safety/oxidized/texinfo";
      inputs.workspace.follows = "workspace";
    };
    wclip = {
      url = "path:./dev/wclip";
      inputs.workspace.follows = "workspace";
    };
  };

  outputs = inputs:
    inputs.workspace ./. {
      inherit inputs;
      # Every directory under here is named after the output it feeds. Its
      # workspace-modules are imported, and nothing inside it is a project, both
      # of which follow from saying this once.
      outputDirs = [./platform/nix];

      # The .age files belong to this tree; the module that reads them does
      # not know where they are until it is told.
      secretsDir = ./platform/nix/secrets;
    };
}

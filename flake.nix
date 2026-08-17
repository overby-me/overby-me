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

  # The workspace half of nix-workspace: it finds the projects, so this file
  # does not list them. Imported by path rather than taken as an input,
  # because nix-workspace is published *from* here: an input would mean
  # publishing before the monorepo could evaluate its own change to it.
  # Integrations: each is a flake owning one upstream and the module that
  # uses it, so a tree takes only what it talks to and its name says what
  # taking it costs. Declaring one is the whole of taking it - the workspace
  # finds every input that exports workspaceModules.
  #
  # Each follows this tree's nixpkgs. Without that an integration builds its
  # upstream against a nixpkgs of its own, which is how zen-browser came to
  # want an ffmpeg the rest of the tree does not have.
  inputs = {
    workspace-darwin = {
      url = "path:./platform/nix/workspace/modules/darwin";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-disko = {
      url = "path:./platform/nix/workspace/modules/disko";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-drowse = {
      url = "path:./platform/nix/workspace/modules/drowse";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-home-manager = {
      url = "path:./platform/nix/workspace/modules/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-modular-skills = {
      url = "path:./platform/nix/workspace/modules/modular-skills";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-nixos-hardware = {
      url = "path:./platform/nix/workspace/modules/nixos-hardware";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-nixos-raspberrypi = {
      url = "path:./platform/nix/workspace/modules/nixos-raspberrypi";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-nix-wallpaper = {
      url = "path:./platform/nix/workspace/modules/nix-wallpaper";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-ragenix = {
      url = "path:./platform/nix/workspace/modules/ragenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-system-manager = {
      url = "path:./platform/nix/workspace/modules/system-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-zen-browser = {
      url = "path:./platform/nix/workspace/modules/zen-browser";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  # The projects this tree publishes, taken as inputs so it builds the flakes
  # it ships and holds them against its own build of the same source. A flake
  # nothing evaluates is the problem these had in the first place, and `?dir=`
  # makes one checkable by hand but nothing runs it. See project-flakes.nix
  # and platform/tangled/publish/README.md.
  #
  # Each `project` input is a path, and each follows this tree's nixpkgs and
  # git-hooks rather than the ones nix-workspace pins: in-tree they should build
  # against what everything else here builds against. A published repo still
  # gets nix-workspace's pins, because that resolution happens in its own lock,
  # not this one.
  #
  # Both `follows` are what keep this free. Without them nix locks a separate
  # node per occurrence even when every one resolves to the same revision:
  # adding these twenty-one put 25 flake-compat nodes and 22 git-hooks nodes
  # in the lock, all at one rev. With them the twenty-one add no fetchable
  # node at all.
  #
  # Not every project: the list is the cheap end, and grows deliberately
  # because each one costs a second build of the crate. The two projects with
  # sibling dependencies are absent for a different reason - they publish as
  # several directories with the crate one level down, a shape their own
  # directory does not have.
  inputs = {
    oxidized-awk = {
      url = "path:./safety/oxidized/awk";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-bash = {
      url = "path:./safety/oxidized/bash";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-binutils = {
      url = "path:./safety/oxidized/binutils";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-bison = {
      url = "path:./safety/oxidized/bison";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-bubblewrap = {
      url = "path:./safety/oxidized/bubblewrap";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-bzip2 = {
      url = "path:./safety/oxidized/bzip2";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-diffutils = {
      url = "path:./safety/oxidized/diffutils";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-file = {
      url = "path:./safety/oxidized/file";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-gcc = {
      url = "path:./safety/oxidized/gcc";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-gzip = {
      url = "path:./safety/oxidized/gzip";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-help2man = {
      url = "path:./safety/oxidized/help2man";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-llvm = {
      url = "path:./safety/oxidized/llvm";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-make = {
      url = "path:./safety/oxidized/make";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-ninja = {
      url = "path:./safety/oxidized/ninja";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-perl = {
      url = "path:./safety/oxidized/perl";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-pipewire = {
      url = "path:./safety/oxidized/pipewire";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-patch = {
      url = "path:./safety/oxidized/patch";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-patchelf = {
      url = "path:./safety/oxidized/patchelf";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-pcre2 = {
      url = "path:./safety/oxidized/pcre2";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-sed = {
      url = "path:./safety/oxidized/sed";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    oxidized-texinfo = {
      url = "path:./safety/oxidized/texinfo";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
    wclip = {
      url = "path:./dev/wclip";
      inputs.project.inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };
  };

  outputs = inputs:
    import ./platform/nix/workspace/workspace.nix ./. {
      inherit inputs;
      # Every directory under here is named after the output it feeds. Its
      # workspace-modules are imported, and nothing inside it is a project, both
      # of which follow from saying this once.
      outputDirs = [./platform/nix];

      # The .age files belong to this tree; the integration that reads them
      # does not know where they are until it is told.
      secretsDir = ./platform/nix/secrets;
    };
}

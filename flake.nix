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

  # The projects this tree publishes are declared one level down, in
  # platform/tangled/publish/checks, and reach this flake as one input. They
  # are inputs because a check builds each project's own flake and holds it
  # against this tree's build of the same source; that is a fact about that
  # check rather than about this tree, and twenty-two lines of it here said so
  # in the wrong place.
  inputs.publish-checks = {
    url = "path:./platform/tangled/publish/checks";
    inputs.workspace.follows = "workspace";
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

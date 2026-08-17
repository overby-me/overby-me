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

  # nix-workspace and the modules taken out of it are declared one level
  # down, in platform/nix, which is the directory that talks to them. This
  # takes that as one input and merges what it carries into its own below, so
  # the workspace still finds every module while the root stops listing them.
  inputs.nix = {
    url = "path:./platform/nix";
    inputs = {
      nixpkgs.follows = "nixpkgs";
      git-hooks.follows = "git-hooks";
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
    inputs.workspace.follows = "nix/workspace";
  };

  outputs = inputs: let
    # What platform/nix talks to, merged in so the workspace finds every
    # module: it scans the inputs it is handed, and these arrive one level
    # down. Merged rather than replaced, so this flake's own inputs still
    # reach the modules that read them.
    all = inputs // inputs.nix.modules;
  in
    all.workspace ./. {
      inputs = all;
      # Every directory under here is named after the output it feeds. Its
      # workspace-modules are imported, and nothing inside it is a project, both
      # of which follow from saying this once.
      outputDirs = [./platform/nix];
    };
}

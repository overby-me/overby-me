{
  description = "Personal Monorepo";

  nixConfig = {
    extra-substituters = ["https://overby-me.cachix.org"];
    extra-trusted-public-keys = ["overby-me.cachix.org-1:dU7qOj5u97QZz98nqnh+Nwait6c+2d2Eq0KTOAXTyp4="];
  };

  inputs = {
    # Build-time environment values, passed in via `--override-input env`
    # (e.g. CI injects PUBLIC_GIT_COMMIT_SHA for apps/wiki). Defaults to an
    # empty file, so local builds fall back to sensible defaults. This no
    # longer carries the working directory: the devshells discover it at
    # runtime instead.
    env = {
      url = "file+file:///dev/null";
      flake = false;
    };

    # Agent Skills
    modular-skills = {
      url = "github:modular/skills";
      flake = false;
    };

    # Nix
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Config support
    nixos-hardware = {
      url = "github:NixOS/nixos-hardware/master";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    disko = {
      # Declarative disk layout for armitas, so nixos-anywhere can partition
      # and format the Surface Pro 11 without anything being typed on it.
      url = "github:nix-community/disko";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nixos-raspberrypi = {
      # Following nixpkgs means eva-00 is built against this flake's nixpkgs
      # rather than the revision nixos-raspberrypi pins. Its binary cache
      # (nixos-raspberrypi.cachix.org) is keyed to that pinned revision, so
      # cached rpi artifacts miss and get rebuilt locally.
      url = "github:nvmd/nixos-raspberrypi/main";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-compat.follows = "flake-compat";
      };
    };
    nix-darwin = {
      url = "github:nix-darwin/nix-darwin/nix-darwin-26.05";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    home-manager = {
      url = "github:nix-community/home-manager/release-26.05";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    system-manager = {
      # Pinned to a nixpkgs revision rather than following one of ours.
      #
      # system-manager imports a curated subset of NixOS modules and re-declares
      # some of their options itself, so it fits a narrow window of nixpkgs
      # rather than a channel. Following the stable 26.05 pulls in modules whose
      # dependencies are not part of that subset (nginx reaching for
      # `security.dhparams`). Following nixpkgs-unstable held up only until
      # unstable moved past the window: 26.11 declares `nix.enable`, which
      # system-manager also declares, and a duplicate declaration is an
      # evaluation error that no amount of local configuration can absorb.
      #
      # The revision below is the one system-manager's own lock names, so it is
      # the combination upstream tests. Dropping the override entirely does not
      # get that for free: a dependency's lock file is never consulted, so Nix
      # would resolve its `nixos-unstable` ref to whatever is newest today and
      # land back on the same error.
      #
      # Bump this when system-manager bumps, not on its own. It is deliberately
      # not a channel ref: floating is what broke it.
      url = "github:numtide/system-manager";
      inputs = {
        nixpkgs.url = "github:NixOS/nixpkgs/61b7c44c4073f0b827768aff0049561b5110ea5a";
        flake-compat.follows = "flake-compat";
        # Do NOT reach into system-manager's `userborn` input to dedupe its
        # dev-time transitive inputs. Overriding `userborn.inputs.*` makes the
        # root flake resolve `userborn` itself, and a sandboxed re-lock that
        # can't read system-manager's flake to learn userborn's URL falls back
        # to a bare `flake:userborn` registry lookup, which fails on builders
        # without registry access (e.g. statichost.eu). Let system-manager
        # carry its own userborn subtree; the extra duplicate dev inputs it
        # pulls in are never built.
      };
    };
    agenix = {
      url = "github:ryantm/agenix";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        home-manager.follows = "home-manager";
        systems.follows = "systems";
      };
    };
    ragenix = {
      url = "github:yaxitech/ragenix";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
        agenix.follows = "agenix";
        rust-overlay.follows = "rust-overlay";
        crane.follows = "crane";
      };
    };

    # Development
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane = {
      url = "github:ipetkov/crane";
    };

    # Apps
    zen-browser = {
      url = "github:0xc000022070/zen-browser-flake";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.home-manager.follows = "home-manager";
    };

    # Styling
    nix-wallpaper = {
      url = "github:lunik1/nix-wallpaper";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
        pre-commit-hooks.follows = "git-hooks";
      };
    };

    # Transitive flake dependencies
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-compat.follows = "flake-compat";
        gitignore.follows = "gitignore";
      };
    };
    gitignore = {
      url = "github:hercules-ci/gitignore.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils = {
      url = "github:numtide/flake-utils";
      inputs.systems.follows = "systems";
    };
    flake-compat = {
      url = "github:edolstra/flake-compat";
    };
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
    };
    hercules-ci-effects = {
      url = "github:hercules-ci/hercules-ci-effects";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-parts.follows = "flake-parts";
    };
    systems.url = "github:nix-systems/default";
    drowse = {
      url = "github:figsoda/drowse";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-parts.follows = "flake-parts";
      };
    };
  };

  # The workspace half of nix-project: it finds the projects, so this file
  # does not list them. Imported by path rather than taken as an input,
  # because nix-project is published *from* here: an input would mean
  # publishing before the monorepo could evaluate its own change to it.
  outputs = inputs:
    import ./platform/nix/project/workspace.nix ./. {
      inherit inputs;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      nixpkgs.config = {
        allowUnfree = true;
      };

      # Every directory under here is named after the output it feeds, and
      # platform/nix/project/outputs.nix reads them. flakelight's own nixDir
      # is switched off: one mechanism, and it is ours.
      outputDirs = [./platform/nix];

      # Every .nix file in it is a flakelight module. nixDir exports them
      # under the directory's own name; importing them is a separate act.
      moduleDirs = [./platform/nix/flakelight-modules];

      # A tool's checks live inside that tool's library rather than in a
      # directory of modules, so these four stay named.
      imports = [
        ./platform/nix/lib/cargo/checks.nix
        ./platform/nix/lib/skylark/checks.nix
        ./platform/nix/lib/buck2/checks.nix
        ./platform/nix/lib/ninja/checks.nix
      ];

      # Every directory holding a default.nix is a project and is imported.
      # platform/nix is not one: it is the nixDir above, and the modules in
      # it are named explicitly.
      projects.exclude = ["platform/nix"];
    };
}

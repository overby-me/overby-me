{
  description = "Personal Monorepo";

  inputs = {
    # Pass env through input
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
    flakelight = {
      url = "github:accelbread/flakelight";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nixos-hardware = {
      url = "github:NixOS/nixos-hardware/master";
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
      # system-manager imports a curated subset of NixOS modules that is
      # version-locked to nixpkgs-unstable (its own default). Following this
      # flake's stable nixpkgs (26.05) pulls in modules whose dependencies
      # aren't part of that subset (e.g. nginx referencing `security.dhparams`),
      # breaking evaluation. Follow nixpkgs-unstable to match its expectations.
      url = "github:numtide/system-manager";
      inputs = {
        nixpkgs.follows = "nixpkgs-unstable";
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
    devenv = {
      url = "github:cachix/devenv";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
        flake-compat.follows = "flake-compat";
        flake-parts.follows = "flake-parts";
        rust-overlay.follows = "rust-overlay";
      };
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
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-compat.follows = "flake-compat";
      inputs.gitignore.follows = "gitignore";
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

  outputs = inputs:
    inputs.flakelight ./. {
      inherit inputs;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      nixpkgs.config = {
        allowUnfree = true;
      };

      imports = [
        ./nix/flake/modules/colmena.nix
        ./nix/flake/modules/darwin.nix
        ./nix/flake/modules/systemConfigs.nix
        ./nix/flake/modules/desktops.nix
        ./nix/flake/modules/devShellNames.nix
        ./nix/flake/modules/devenvConfigurations.nix
        ./nix/flake/modules/devenvModules.nix
        ./nix/flake/modules/filterUnsupported.nix
        ./nix/flake/modules/hardware.nix
        ./nix/flake/modules/homeConfigurations.nix
        ./nix/flake/modules/lib.nix
        ./nix/flake/modules/perSystemLib.nix
        ./nix/flake/modules/secrets.nix
        ./nix/flake/modules/users.nix
        ./nix/flake/modules/zedExtensions.nix

        ./ironclaw/bluesky
        ./ironclaw/calendar
        ./ironclaw/contacts
        ./ironclaw/mail
        ./ironclaw/matrix
        ./ironclaw/pixtral
        ./ironclaw/searxng
        ./ironclaw/signal

        ./mojo/gui
        ./mojo/wasm
        ./mojo/zed

        ./nickel/workspace
        ./nickel/zed

        ./nushell/plugin-tramp

        ./rust/awk
        ./rust/bash
        ./rust/binutils
        ./rust/bison
        ./rust/bubblewrap
        ./rust/bzip2
        ./rust/cachix
        ./rust/curl
        ./rust/diffutils
        ./rust/direnv
        ./rust/file
        ./rust/flatpak
        ./rust/gcc
        ./rust/grep
        ./rust/gzip
        ./rust/h26xtoav1
        ./rust/help2man
        ./rust/make
        ./rust/meson
        ./rust/nixos
        ./rust/ninja
        ./rust/nixpkgs
        ./rust/patch
        ./rust/patchelf
        ./rust/pcre2
        ./rust/perl
        ./rust/pkg-config
        ./rust/sed
        ./rust/systemd
        ./rust/tar
        ./rust/texinfo
        ./rust/wclip
        ./rust/xz

        ./slides

        ./tangled/cli
        ./tangled/spindle-nix-engine

        ./web/homepage
        ./web/wiki
        ./web/wiki-dioxus
      ];
      nixDirAliases = {
        packages = ["pkgs"];
        flakelightModules = ["flake/modules"];
        nixosConfigurations = ["nixos/config"];
        nixosModules = ["nixos/modules"];
        darwinConfigurations = ["darwin/config"];
        darwinModules = ["darwin/modules"];
        systemConfigs = ["system-manager/config"];
        systemModules = ["system-manager/modules"];
        homeConfigurations = ["home-manager/config"];
        homeModules = ["home-manager/modules"];
        devenvConfiguration = ["devenv/config"];
        devenvModules = ["devenv/modules"];
        users = ["home-manager/users"];
        hardware = ["nixos/hardware"];
        desktops = ["nixos/desktops"];
        withOverlays = ["with-overlays"];
      };
    };
}

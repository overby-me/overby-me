{
  description = "Personal Monorepo";

  inputs = {
    # Pass env through input
    env = {
      url = "file+file:///dev/null";
      flake = false;
    };

    # Nix
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixos-unstable";
    lix = {
      url = "git+https://git.lix.systems/lix-project/lix?ref=refs/tags/2.95.0";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-compat.follows = "flake-compat";
        pre-commit-hooks.follows = "git-hooks";
      };
    };

    # Config support
    flakelight = {
      url = "github:accelbread/flakelight";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nixos-hardware.url = "github:NixOS/nixos-hardware/master";
    nixos-raspberrypi = {
      url = "github:nvmd/nixos-raspberrypi/main";
    };
    home-manager = {
      url = "github:nix-community/home-manager/release-25.11";
      inputs.nixpkgs.follows = "nixpkgs";
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
      };
    };

    # XR
    non-spatial-input = {
      url = "github:StardustXR/non-spatial-input";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        crane.follows = "crane";
      };
    };

    # Apps
    zen-browser = {
      url = "github:0xc000022070/zen-browser-flake";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.home-manager.follows = "home-manager";
    };
    zed = {
      url = "github:zed-industries/zed";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        crane.follows = "crane";
        rust-overlay.follows = "rust-overlay";
        flake-parts.follows = "flake-parts";
      };
    };
    tangled = {
      url = "git+https://tangled.org/tangled.org/core";
      inputs = {
        nixpkgs.follows = "nixpkgs";
      };
    };
    peon-ping = {
      url = "github:PeonPing/peon-ping";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
      };
    };
    nxv = {
      url = "github:jamesbrink/nxv";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
        rust-overlay.follows = "rust-overlay";
        crane.follows = "crane";
      };
    };

    # Styling
    spicetify-nix = {
      url = "github:Gerg-L/spicetify-nix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.systems.follows = "systems";
    };
    catppuccin = {
      url = "github:catppuccin/nix/release-25.11";
      inputs.nixpkgs.follows = "nixpkgs";
    };
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

  outputs = inputs: let
    # Extend nixpkgs lib with all builtins so that flakelight's evalModules
    # propagates the merged lib to every module.  This lets all flakelight
    # modules (and anything using pkgs.lib) write lib.readDir, lib.fromJSON,
    # lib.unsafeDiscardStringContext, etc. instead of reaching for builtins.
    extendedNixpkgs =
      inputs.nixpkgs
      // {
        lib = inputs.nixpkgs.lib.extend (_: prev: builtins // prev);
      };
    flakelight = import "${inputs.flakelight}" (inputs.flakelight.inputs
      // {
        nixpkgs = extendedNixpkgs;
        self = inputs.flakelight;
      });
  in
    flakelight.mkFlake ./. {
      inherit inputs;
      nixpkgs.config = {
        allowUnfree = true;
      };
      flakelight.builtinFormatters = false;

      imports = [
        ./nix/modules/flakelight/lib.nix
        ./nix/modules/flakelight/secrets.nix
        ./nix/modules/flakelight/users.nix
        ./nix/modules/flakelight/hardware.nix
        ./nix/modules/flakelight/desktops.nix
        ./nix/modules/flakelight/devenvModules.nix
        ./nix/modules/flakelight/devenvConfigurations.nix
        ./nix/modules/flakelight/devShellNames.nix
        ./nix/modules/flakelight/colmena.nix
        ./nix/modules/flakelight/perSystemLib.nix
        ./nix/modules/flakelight/zedExtensions.nix

        ./nickel/workspace

        ./web/homepage
        ./web/homepage-dioxus
        ./web/wiki

        ./mojo/wasm
        ./mojo/gui
        ./mojo/zed
        ./nickel/zed

        ./rust/nixos
        ./rust/nixpkgs
        ./rust/systemd
        ./rust/pkg-config
        ./nushell/plugin-tramp

        ./slides

        ./rust/awk
        ./rust/bash
        ./rust/binutils
        ./rust/bison
        ./rust/bzip2
        ./rust/gcc
        ./rust/cachix
        ./rust/direnv
        ./rust/grep
        ./rust/sed
        ./rust/make
        ./rust/meson
        ./rust/gzip
        ./rust/h264toav1
        ./rust/patch
        ./rust/patchelf
        ./rust/strip
        ./rust/tar
        ./rust/xz
        ./rust/diffutils
        ./rust/file
        ./rust/texinfo
        ./rust/help2man
        ./rust/curl
        ./rust/bubblewrap
        ./rust/flatpak
        ./ironclaw/matrix
        ./ironclaw/bluesky
        ./ironclaw/signal
        ./ironclaw/searxng
        ./ironclaw/mail
        ./ironclaw/calendar
        ./ironclaw/contacts
        ./ironclaw/pixtral

        ./tangled/cli
        ./tangled/spindle-nix-engine
      ];
      nixDirAliases = {
        packages = ["pkgs"];
        flakelightModules = ["modules/flakelight"];
        nixosConfigurations = ["config/nixos"];
        nixosModules = ["modules/nixos"];
        homeConfigurations = ["config/home-manager"];
        homeModules = ["modules/home-manager"];
        devenvConfiguration = ["config/devenv"];
        devenvModules = ["modules/devenv"];
        secrets = ["config/secrets"];
        users = ["config/users"];
        hardware = ["config/hardware"];
        desktops = ["config/desktops"];
        withOverlays = ["config/with-overlays"];
      };
    };
}

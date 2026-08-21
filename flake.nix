{
  description = "Monorepo";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

    # The framework this flake calls. The integrations are optional inputs on
    # it, so overriding one here is what enables it.
    workspace = {
      url = "git+https://tangled.org/overby.me/nix-workspace";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        # Upstream ships no flake, so the framework carries one holding the pin.
        secretspec.url = "git+https://tangled.org/overby.me/nix-workspace?dir=upstreams/secretspec";

        nix-darwin = {
          url = "github:nix-darwin/nix-darwin/nix-darwin-26.05";
          inputs.nixpkgs.follows = "nixpkgs";
        };
        home-manager = {
          url = "github:nix-community/home-manager/release-26.05";
          inputs.nixpkgs.follows = "nixpkgs";
        };
        system-manager = {
          url = "github:numtide/system-manager";
          inputs.userborn.inputs.pre-commit-hooks-nix.follows = "workspace/git-hooks";
        };
      };
    };

    # Forced by this tree: zed's wasip2 toolchain and ironclaw take rust-bin.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  inputs.disko = {
    url = "github:nix-community/disko";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  # Direct upstream, not an integration: hosts import its nixosModules by name.
  inputs.nixos-hardware = {
    url = "github:NixOS/nixos-hardware/master";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  # Direct upstreams, not integrations: their default package lands in pkgs.
  inputs.nix-wallpaper = {
    url = "github:lunik1/nix-wallpaper";
    inputs.nixpkgs.follows = "nixpkgs";
    inputs.pre-commit-hooks.follows = "workspace/git-hooks";
  };
  inputs.zen-browser = {
    # Not the default branch: zen tracks nixpkgs-unstable, and its HEAD package
    # calls for ffmpeg_9, which 26.05 does not carry. Unpin when 26.05 does.
    url = "github:0xc000022070/zen-browser-flake/945efbc704b7f8c1731a922aabbc5d95edc9eb74";
    inputs.nixpkgs.follows = "nixpkgs";
    inputs.home-manager.follows = "workspace/home-manager";
  };

  # This tree's nix configuration.
  inputs.nix-config = {
    url = "path:./platform/nix/config";
    inputs = {
      workspace.follows = "workspace";
      # This tree's own copies: fetching the published ones too would declare
      # their home-manager options twice, which is a conflict, not an override.
      nushell-plugin-tramp.url = "path:./dev/nushell/plugin-tramp";
      tangled-spindle-nix-engine.url = "path:./platform/tangled/spindle-nix-engine";
      nix-packages.follows = "nix-packages";
    };
  };

  # The build systems. Its port inputs point at this tree's own copies.
  inputs.nix-lib = {
    url = "path:./platform/nix/lib";
    inputs = {
      workspace.follows = "workspace";
      wclip.url = "path:./dev/wclip";
      oxidized-xz.url = "path:./safety/oxidized/xz";
      oxidized-ninja.url = "path:./safety/oxidized/ninja";
    };
  };

  # Packages built from source, separate so either can be taken alone.
  inputs.nix-packages = {
    url = "path:./platform/nix/packages";
    inputs.workspace.follows = "workspace";
  };

  # The differential check: each project's own flake against this tree's build
  # of the same source. One level down, so this file carries one input.
  inputs.publish-checks = {
    url = "path:./platform/tangled/publish/checks";
    inputs.workspace.follows = "workspace";
  };

  outputs = inputs:
    inputs.workspace {
      inherit inputs;
    };
}

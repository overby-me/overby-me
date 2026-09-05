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

  # This tree's nix configuration.
  inputs.nix-config = {
    url = "path:./platform/nix/config";
    inputs = {
      workspace.follows = "workspace";
      # This tree's own copies: fetching the published ones too would declare
      # their home-manager options twice, which is a conflict, not an override.
      # The same holds for what those copies themselves fetch, so their
      # nix-lib has to land on this tree's copy as well.
      nushell-plugin-tramp = {
        url = "path:./dev/nushell/plugin-tramp";
        inputs.nix-lib.follows = "nix-lib";
      };
      tangled-spindle-nix-engine = {
        url = "path:./platform/tangled/spindle-nix-engine";
        inputs.nix-lib.follows = "nix-lib";
      };
      nix-packages.follows = "nix-packages";
    };
  };

  # The build systems. Its ninja input points at this tree's own copy.
  inputs.nix-lib = {
    url = "path:./platform/nix/lib";
    inputs = {
      workspace.follows = "workspace";
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

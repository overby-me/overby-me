{
  description = "Monorepo";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

    # The framework this flake calls. Not a module: it builds the flake.
    #
    # The integration upstreams are set here rather than declared beside the
    # other inputs. The framework declares each of them as an optional input
    # defaulting to a stub, so enabling one is overriding it here, and this
    # tree names a pin once instead of once per flake that needs it.
    #
    # Each override replaces the framework's declaration whole, so each says
    # where its own nixpkgs comes from: `follows` in a nested override
    # resolves against this flake, so these land on the nixpkgs above.
    #
    # secretspec points back at this same repo: upstream is a plain Cargo
    # repo with no flake of its own, so the framework ships the flake it
    # lacks and that wrapper holds the pin.
    workspace = {
      url = "git+https://tangled.org/overby.me/nix-workspace";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        secretspec.url = "git+https://tangled.org/overby.me/nix-workspace?dir=upstreams/secretspec";

        nix-darwin = {
          url = "github:nix-darwin/nix-darwin/nix-darwin-26.05";
          inputs.nixpkgs.follows = "nixpkgs";
        };
        home-manager = {
          url = "github:nix-community/home-manager/release-26.05";
          inputs.nixpkgs.follows = "nixpkgs";
        };
        # system-manager fits a narrow window rather than a channel (it
        # re-declares options from a curated NixOS module subset), so both
        # the upstream and its nested nixpkgs are pinned to revisions known
        # to agree: newer system-manager drops the nix.* module this tree's
        # configs set.
        system-manager = {
          url = "github:numtide/system-manager/48d47346e0c6ad05b6c869ea92649c47723d1cfc";
          inputs.nixpkgs.url = "github:NixOS/nixpkgs/61b7c44c4073f0b827768aff0049561b5110ea5a";
          # userborn's devshell tooling, never evaluated here; deduped onto
          # the copy the framework already carries.
          inputs.userborn.inputs.pre-commit-hooks-nix.follows = "workspace/git-hooks";
        };
      };
    };

    # Optional to nix-config, forced by this tree: zed's wasip2 toolchain
    # and ironclaw's build take rust-bin from here.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  inputs.disko = {
    url = "github:nix-community/disko";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  # Direct upstream, not an integration: hosts import its nixosModules by
  # name and there is no module logic to carry, so declaring it is the
  # whole of having it.
  inputs.nixos-hardware = {
    url = "github:NixOS/nixos-hardware/master";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  # Direct upstreams, not integrations: an input that exports a default
  # package lands in pkgs under its own name, so declaring it is the whole
  # of having it. zen-browser is also read for its home module.
  inputs.nix-wallpaper = {
    url = "github:lunik1/nix-wallpaper";
    inputs.nixpkgs.follows = "nixpkgs";
    # Their devshell tooling, never evaluated here; deduped onto the copy
    # the framework already carries.
    inputs.pre-commit-hooks.follows = "workspace/git-hooks";
  };
  inputs.zen-browser = {
    # Pinned to the rev the retired integration had locked: zen tracks
    # nixpkgs-unstable at HEAD (its package wants ffmpeg_9), and this rev is
    # the one known to build against this tree's 26.05.
    url = "github:0xc000022070/zen-browser-flake/945efbc704b7f8c1731a922aabbc5d95edc9eb74";
    inputs.nixpkgs.follows = "nixpkgs";
    inputs.home-manager.follows = "workspace/home-manager";
  };

  # This tree's nix configuration. Taking it is the whole of using it: the
  # workspace imports every input that exports a module, so this file says
  # neither what is in there nor what it needs.
  inputs.nix-config = {
    url = "path:./platform/nix/config";
    inputs = {
      workspace.follows = "workspace";
      # Pointed at this tree's own copies, so a change to one is what the
      # hosts get rather than whatever was last published - and so the module
      # arrives once. Both copies declare the same home-manager options, and
      # two paths to one option is a duplicate declaration, not an override.
      nushell-plugin-tramp.url = "path:./dev/nushell/plugin-tramp";
      tangled-spindle-nix-engine.url = "path:./platform/tangled/spindle-nix-engine";
      nix-packages.follows = "nix-packages";
    };
  };

  # The build systems. Its three port inputs are pointed at this tree's own
  # copies, so a change to one is what the check builds rather than whatever
  # was last published.
  inputs.nix-lib = {
    url = "path:./platform/nix/lib";
    inputs = {
      workspace.follows = "workspace";
      wclip.url = "path:./dev/wclip";
      oxidized-xz.url = "path:./safety/oxidized/xz";
      oxidized-ninja.url = "path:./safety/oxidized/ninja";
    };
  };

  # The packages built from source, separate from the configuration so either
  # can be taken without the other.
  inputs.nix-packages = {
    url = "path:./platform/nix/packages";
    inputs.workspace.follows = "workspace";
  };

  # The published projects, as inputs, because the differential check builds
  # each project's own flake and holds it against this tree's build of the same
  # source. Declared one level down so this file does not carry twenty-two of
  # them.
  inputs.publish-checks = {
    url = "path:./platform/tangled/publish/checks";
    inputs.workspace.follows = "workspace";
  };

  outputs = inputs:
    inputs.workspace {
      inherit inputs;
    };
}

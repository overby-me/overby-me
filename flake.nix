{
  description = "Monorepo";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

    # The framework this flake calls. Not a module: it builds the flake.
    workspace = {
      url = "git+https://tangled.org/overby.me/nix-workspace";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  # The integrations this tree's hosts force. An integration is enabled by
  # whoever declares it, and declaring is the whole of enabling - nix-config
  # deliberately declares none, so a consumer of it inherits no pin it does
  # not name, and the tree that evaluates the hosts names them instead.
  inputs.workspace-darwin = {
    url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/darwin";
    inputs.workspace.follows = "workspace";
  };
  inputs.disko = {
    url = "github:nix-community/disko";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  inputs.workspace-home-manager = {
    url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/home-manager";
    inputs.workspace.follows = "workspace";
  };
  inputs.workspace-nixos-hardware = {
    url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/nixos-hardware";
    inputs.workspace.follows = "workspace";
  };
  inputs.nixos-raspberrypi = {
    url = "github:nvmd/nixos-raspberrypi/main";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  # Direct upstreams, not integrations: an input that exports a default
  # package lands in pkgs under its own name, so declaring it is the whole
  # of having it. zen-browser is also read for its home module.
  inputs.nix-wallpaper = {
    url = "github:lunik1/nix-wallpaper";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  inputs.workspace-ragenix = {
    url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/ragenix";
    inputs.workspace.follows = "workspace";
  };
  inputs.workspace-system-manager = {
    url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/system-manager";
    inputs.workspace.follows = "workspace";
  };
  inputs.zen-browser = {
    # Pinned to the rev the retired integration had locked: zen tracks
    # nixpkgs-unstable at HEAD (its package wants ffmpeg_9), and this rev is
    # the one known to build against this tree's 26.05.
    url = "github:0xc000022070/zen-browser-flake/945efbc704b7f8c1731a922aabbc5d95edc9eb74";
    inputs.nixpkgs.follows = "nixpkgs";
    inputs.home-manager.follows = "workspace-home-manager/home-manager";
  };

  # This tree's nix configuration. Taking it is the whole of using it: the
  # workspace imports every input that exports a module, so this file says
  # neither what is in there nor what it needs.
  inputs.nix-config = {
    url = "path:./platform/nix/config";
    inputs.workspace.follows = "workspace";
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

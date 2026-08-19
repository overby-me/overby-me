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
    inputs.nixpkgs.follows = "nixpkgs";
  };
  inputs.workspace-disko = {
    url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/disko";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  inputs.workspace-home-manager = {
    url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/home-manager";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  inputs.workspace-nixos-hardware = {
    url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/nixos-hardware";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  inputs.workspace-nixos-raspberrypi = {
    url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/nixos-raspberrypi";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  inputs.workspace-nix-wallpaper = {
    url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/nix-wallpaper";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  inputs.workspace-ragenix = {
    url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/ragenix";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  inputs.workspace-system-manager = {
    url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/system-manager";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  inputs.workspace-zen-browser = {
    url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/zen-browser";
    inputs.nixpkgs.follows = "nixpkgs";
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

  # nix-config and nix-packages both carry rust-overlay, and both hand their
  # inputs to the module system, which takes one definition per name. This
  # names the survivor; without it the tree fails evaluation with conflicting
  # definitions of `inputs.rust-overlay`.
  inputs.rust-overlay.follows = "nix-config/rust-overlay";

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

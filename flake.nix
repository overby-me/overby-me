{
  description = "Monorepo";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

    # What this flake calls. The modules taken out of the same repo belong to
    # platform/nix/config, which is what talks to them; this one is not a
    # module and nothing here talks to it - it builds the flake.
    workspace = {
      url = "git+https://tangled.org/overby.me/nix-workspace";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  # This tree's nix configuration, as a workspace of its own: it carries the
  # outputs under platform/nix/config and the modules that directory talks to, and
  # exports them as one module. Taking it is the whole of using it, because
  # the workspace imports every input that exports a module - so this file
  # says neither what is in there nor what it needs.
  inputs.nix-config = {
    url = "path:./platform/nix/config";
    inputs.workspace.follows = "workspace";
  };

  # The build systems, as a workspace: every port in this tree builds with a
  # `lib` from in there, and none of them is a machine configuration. Reaching
  # it through the flake that also carries eight NixOS hosts had that
  # backwards, and it was most of that flake.
  #
  # The three ports are its fixtures - two Cargo workspaces the resolver is
  # tested against, and the ninja rewrite whose `graph-json` its build graph
  # extraction runs - pointed at this tree's own copies so a change to one is
  # what the check builds.
  inputs.nix-lib = {
    url = "path:./platform/nix/lib";
    inputs = {
      workspace.follows = "workspace";
      wclip.url = "path:./dev/wclip";
      oxidized-xz.url = "path:./safety/oxidized/xz";
      oxidized-ninja.url = "path:./safety/oxidized/ninja";
    };
  };

  # The packages built from source, which are a workspace rather than a
  # directory of the configuration: what they need is a pkg-config, and what
  # the configuration needs is eight hosts' worth of modules. Splitting them
  # is what lets either be taken without the other.
  inputs.nix-packages = {
    url = "path:./platform/nix/packages";
    inputs.workspace.follows = "workspace";
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
    };
}

{
  description = "Monorepo";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

    # What this flake calls. The modules taken out of the same repo belong to
    # platform/nix, which is what talks to them; this one is not a module and
    # nothing here talks to it - it builds the flake.
    workspace = {
      url = "git+https://tangled.org/overby.me/nix-workspace";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  # This tree's nix configuration, as a workspace of its own: it carries the
  # outputs under platform/nix and the modules that directory talks to, and
  # exports them as one module. Taking it is the whole of using it, because
  # the workspace imports every input that exports a module - so this file
  # says neither what is in there nor what it needs.
  inputs.nix-config = {
    url = "path:./platform/nix";
    inputs = {
      nixpkgs.follows = "nixpkgs";
      workspace.follows = "workspace";
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
    inputs.workspace.follows = "workspace";
  };

  outputs = inputs:
    inputs.workspace ./. {
      inherit inputs;

      # Nothing in there is a project. It used to follow from `outputDirs`,
      # which excluded what it scanned; now that the scanning belongs to that
      # directory, the exclusion has to be said. Without it discovery walks
      # platform/nix, finds every default.nix under it, and imports as a
      # project module one that builds its own `imports` from `pkgs` - which
      # is a loop, and surfaces as infinite recursion nowhere near here.
      workspaces.exclude = ["platform/nix"];
    };
}

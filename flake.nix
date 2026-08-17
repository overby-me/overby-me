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

  # The modules taken out of nix-workspace are declared one level down, in
  # platform/nix, which is the directory that talks to them. This takes that
  # as one input and merges what it carries into its own below, so the
  # workspace still finds every module while this file stops listing them.
  inputs.nix = {
    url = "path:./platform/nix";
    inputs.nixpkgs.follows = "nixpkgs";
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
      # What platform/nix talks to, merged in so the workspace finds every
      # module: it scans the inputs it is handed, and those arrive one level
      # down. Merged rather than replaced, so this flake's own inputs still
      # reach the modules that read them.
      inputs = inputs // inputs.nix.modules;
      # Every directory under here is named after the output it feeds. Its
      # workspace-modules are imported, and nothing inside it is a project, both
      # of which follow from saying this once.
      outputDirs = [./platform/nix];
    };
}

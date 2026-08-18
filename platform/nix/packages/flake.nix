# The packages this tree builds that nixpkgs does not have.
#
# A workspace of its own so a tree can take the packages without taking eight
# NixOS hosts and a secrets directory with them.
#
# The directory inside is named again because `packages/<name>.nix` becomes
# `packages.<name>`: this flake is the workspace, `packages/` is the output.
#
# Six of these link native libraries and briefly took the tree's own pkg-config
# rewrite, which made a package set depend on a port of a build tool.
{
  description = "Packages built from source, for a tree that wants them without the configuration around them";

  inputs = {
    # nixpkgs comes through the framework rather than being declared here.
    # Declaring both means every consumer has to follow both, and forgetting
    # the second builds this against a nixpkgs of its own, silently - which is
    # how zen-browser came to want an ffmpeg the rest of the tree did not have.
    workspace.url = "git+https://tangled.org/overby.me/nix-workspace";
  };

  outputs = inputs: {workspaceModule = inputs.workspace.workspaceIn ./. inputs;};
}

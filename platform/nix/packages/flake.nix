# The packages this tree builds that nixpkgs does not have.
#
# A workspace of its own rather than a directory of the configuration, for the
# same reason the configuration is one: what it needs is not what the
# configuration needs, and saying so here is what lets a tree take the
# packages without taking eight NixOS hosts and a secrets directory with them.
#
# `packages/<name>.nix` becomes `packages.<name>`, which is the framework's
# rule everywhere - the path is the address. That is why the directory inside
# is named again: this flake is the workspace, and `packages/` is the output
# it feeds.
{
  description = "Packages built from source, for a tree that wants them without the configuration around them";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

    # For `outputsIn`, which turns this directory into a module. The consuming
    # flake follows its own copy onto this, so there is one framework.
    workspace = {
      url = "git+https://tangled.org/overby.me/nix-workspace";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # The one port these reach for. Six of them build Rust that links native
    # libraries, and take it as the pkg-config on the build side rather than
    # nixpkgs' - which is the point of having rewritten it. It arrived as an
    # ambient package while these lived beside the tree that builds it; a
    # workspace has to name what it uses.
    oxidized-pkg-config = {
      url = "git+https://tangled.org/overby.me/oxidized-pkg-config";
      inputs.workspace.follows = "workspace";
    };
  };

  # A module, like the configuration beside it: a tree that has this input has
  # the packages, and does not have to name the directory they are in.
  outputs = inputs: {
    workspaceModule = {
      imports = [(inputs.workspace.outputsIn ./.)];

      # mkDefault, because the consuming tree names nixpkgs too and `inputs`
      # takes one definition per name: handing them over plainly is a
      # collision rather than a merge.
      inputs = inputs.nixpkgs.lib.mapAttrs (_: inputs.nixpkgs.lib.mkDefault) (removeAttrs inputs ["self"]);

      outputDirs = [./.];
    };
  };
}

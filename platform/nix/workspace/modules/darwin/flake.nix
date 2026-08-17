# nix-darwin, and the module that builds darwin systems with it.
#
# An integration owns both halves of talking to something: the module that
# knows how to use it, and the pin of what it uses. A tree that takes this
# flake gets both, and a tree that does not never fetches either.
#
# That is the only way a pin can be selective. Flake inputs are resolved from
# a flake.nix before anything evaluates, so a pin can belong to a flake or to
# nothing; making each integration a flake is what lets one be chosen. One
# upstream each, so the name says what taking it costs.
#
# The shape is flakelight-darwin's, which reached it first.
{
  description = "nix-darwin, and the module that builds darwin systems with it";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

    nix-darwin = {
      url = "github:nix-darwin/nix-darwin/nix-darwin-26.05";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs: {
    workspaceModule = {lib, ...}: {
      # Identity for the module system, so two copies of this module are
      # one. A bare function has none, and the same one imported twice
      # declares its options twice. See nix-workspace/module.nix.
      key = "nix-workspace/modules/darwin";

      imports = [
        ./module.nix
      ];

      # Handed to the consumer, which therefore never declares it. mkDefault,
      # so a consumer that wants a different one can still say so.
      inputs.nix-darwin = lib.mkDefault inputs.nix-darwin;
    };
  };
}

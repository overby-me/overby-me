# system-manager, for a host that is not NixOS.
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
  description = "system-manager, for a host that is not NixOS";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

    # Pinned to a revision rather than following ours: system-manager imports
    # a curated subset of NixOS modules and re-declares some of their options,
    # so it fits a narrow window of nixpkgs rather than a channel. Following
    # stable pulls in modules whose dependencies are outside that subset;
    # following unstable held up only until unstable declared nix.enable,
    # which system-manager also declares. This is the revision its own lock
    # names, so it is the combination upstream tests.
    system-manager = {
      url = "github:numtide/system-manager";
      inputs.nixpkgs.url = "github:NixOS/nixpkgs/61b7c44c4073f0b827768aff0049561b5110ea5a";
    };
  };

  outputs = inputs: {
    workspaceModule = {lib, ...}: {
      # Identity for the module system, so two copies of this module are
      # one. A bare function has none, and the same one imported twice
      # declares its options twice. See nix-workspace/module.nix.
      key = "nix-workspace/modules/system-manager";

      imports = [
        ./systemConfigs.nix
      ];

      # Handed to the consumer, which therefore never declares it. mkDefault,
      # so a consumer that wants a different one can still say so.
      inputs.system-manager = lib.mkDefault inputs.system-manager;
    };
  };
}

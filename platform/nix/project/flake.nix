# The shared build for repos published out of the overby.me monorepo.
#
# Every such repo is one filtered directory of that monorepo and needs the
# same three things: a plain-nixpkgs build of its crate, a devshell carrying
# the gates the monorepo holds it to, and a formatter. This exports that as a
# flakelight module so each repo states only what is different about it,
# rather than carrying its own copy of one hook list.
#
# It is itself published, at tangled.org/overby.me/nix-standalone and
# github.com/overby-me/nix-standalone, and the monorepo is the source of
# truth for it as for everything else here.
{
  description = "The shared flakelight module for projects published from the overby.me monorepo";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/fcb8fcd6bf2d0adecae5bd491afaaaf8311b758d";

    flakelight = {
      url = "github:accelbread/flakelight";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Re-exported through this module, so a consuming repo does not declare it.
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs:
    inputs.flakelight ./. {
      inherit inputs;
      systems = ["x86_64-linux" "aarch64-linux" "aarch64-darwin"];

      flakelightModule = import ./module.nix {gitHooks = inputs.git-hooks;};

      formatter = pkgs: pkgs.alejandra;
    };
}

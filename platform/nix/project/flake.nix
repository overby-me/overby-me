# The shared build for repos published out of the overby.me monorepo.
#
# Every such repo is one filtered directory of that monorepo and needs the
# same three things: a plain-nixpkgs build of its crate, a devshell carrying
# the gates the monorepo holds it to, and a formatter. This exports that as a
# flakelight module so each repo states only what is different about it,
# rather than carrying its own copy of one hook list.
#
# It is itself published, at tangled.org/overby.me/nix-project and
# github.com/overby-me/nix-project, and the monorepo is the source of
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

  outputs = inputs: let
    module = import ./module.nix {gitHooks = inputs.git-hooks;};
  in
    inputs.flakelight ./. {
      inherit inputs;
      systems = ["x86_64-linux" "aarch64-linux" "aarch64-darwin"];

      flakelightModule = module;

      # The other half: `project` builds one published unit, `workspace`
      # finds the tree of them. It closes over nothing (see workspace.nix),
      # so it is exported as a plain function rather than through the
      # functor, and a monorepo can import it by path instead of taking this
      # flake as an input.
      outputs.workspace = import ./workspace.nix;

      # Callable, so a published repo's whole flake is one call rather than a
      # copy of the same three inputs, the same import and the same follows.
      # The inputs closed over here are this flake's own, which is what makes
      # nixpkgs identical across every published repo by construction: there
      # is one lock to pin it in, not thirty-eight to keep in step.
      #
      #   outputs = inputs: inputs.project ./. { name = "oxidized-sed"; ... };
      #
      # Everything in that set is a `project` option, except withOverlays,
      # which a project needing an overlay of its own passes through: the
      # overlay's flake has to be its input, not ours, or every repo would
      # fetch it.
      functor = _: root: cfg:
        inputs.flakelight root {
          inherit inputs;
          imports = [module];
          project = (removeAttrs cfg ["withOverlays"]) // {inherit root;};
          withOverlays = cfg.withOverlays or [];
        };

      formatter = pkgs: pkgs.alejandra;
    };
}

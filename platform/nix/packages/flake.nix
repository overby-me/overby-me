# The packages this tree builds that nixpkgs does not have.
#
# A workspace of its own so a tree can take the packages without taking eight
# NixOS hosts and a secrets directory with them.
#
# The packages sit at the root - `<name>.nix` becomes `packages.<name>` -
# because this workspace is one output, said by pointing `packages` at the
# directory itself instead of holding a packages/packages/ stutter.
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

    # Three packages build with a nightly toolchain and take `rust-bin` from
    # this. Declared here so the packages travel with the input they need: a
    # consumer that takes only this workspace used to fail evaluation at
    # forkfs with "called without required argument rust-bin". The monorepo
    # pins it to nix-config's copy with a root-level follows, because two
    # workspaces handing over the same input name is a definition conflict,
    # not a merge.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };
  };

  outputs = inputs:
    inputs.workspace {
      inherit inputs;
      packages = ./.;
    };
}

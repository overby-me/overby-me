# Builds the default devShell from pure devshell modules (platform/nix/config/devshell):
# the shared tooling, the git hooks and the root config files, and nothing else.
#
# It used to fold every other named shell in through `inputsFrom`, so
# `nix develop` carried the union of twenty project shells. That made the
# default shell only as portable as the least portable of them - one shell
# naming a package this system cannot build turned plain `nix develop` into a
# checkMeta throw - and it put every project's toolchain on one PATH, where
# two of them shadow each other. A project's shell is now its own:
# `nix develop .#<project>`.
{
  lib,
  inputs,
  ...
}: let
  mkDevShell = import ../devshell/lib/mkDevShell.nix {inherit lib inputs;};
in {
  config.devShells.default = pkgs:
    mkDevShell pkgs [
      ../devshell/modules/common.nix
      ../devshell/modules/git-hooks.nix
      ../devshell/modules/configs/default.nix
    ];
}

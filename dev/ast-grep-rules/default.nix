# The check lives one level down, in ./check, because the published repo needs
# it there: discovery never descends into its own root, so a repo whose root
# held the definition would build nothing. Here this directory is what is
# discovered, and the shim hands it what the published walk finds on its own.
import ./check/default.nix

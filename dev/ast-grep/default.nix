# The check lives one level down, in ./check, because the published repo
# needs it there: the workspace framework never treats a workspace root as a
# project, so a repo whose root held the definition would build nothing. In
# the monorepo this directory is the discovered project, and this shim hands
# it the same definition the published repo's walk finds on its own.
import ./check/default.nix

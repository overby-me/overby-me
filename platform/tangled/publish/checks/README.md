# platform/tangled/publish/checks

The published projects, as inputs, so the root flake does not carry them.

Each project this monorepo publishes also ships its own `flake.nix`, and
[`workspace-modules/project-flakes.nix`](../../../nix/workspace-modules/project-flakes.nix)
builds each one and holds it against this tree's own build of the same source.
Doing that needs every project as a flake input, and nix resolves `inputs` from
a `flake.nix` before anything evaluates - so an input can only live in a flake.

It need not live in *the* flake. Twenty-two of them in the root said nothing
about this tree except that a check exists somewhere; here they sit beside the
check that wants them, and the root takes one input instead of twenty-two.

## The list is this file

Nothing restates which projects are compared. The check iterates
`published`, which is this flake's inputs minus itself and the framework, so
adding a project is one entry here and removing one is one deletion.

## Two things that will bite

**The relative paths only resolve through the repo root.** `path:../../../../safety/oxidized/awk`
is resolved against the filesystem root when this flake is evaluated on its
own, so `nix eval path:./platform/tangled/publish/checks#published` fails with
*path '/safety/oxidized/awk/flake.nix' does not exist*. Reached as an input of
the root flake, or as `nix eval '.?dir=platform/tangled/publish/checks#published'`,
it resolves inside the tree.

**A path input's declared inputs are frozen in the parent lock.** The content
is live - there is no `narHash` - but the set of input *names* a sub-flake
declares is recorded, so renaming one here is not noticed until those nodes are
dropped from the lock. `nix flake lock` alone will not do it.

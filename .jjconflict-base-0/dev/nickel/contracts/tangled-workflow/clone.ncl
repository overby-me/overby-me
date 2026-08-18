# Clone configuration contract for tangled workflows
#
# Controls how (or whether) the repository is cloned into the workspace
# before steps execute.
{
  CloneConfig = {
    skip
      | Bool
      | doc "Skip cloning entirely"
      | default
      = false,

    depth
      | Number
      | doc "Shallow clone depth (default: 1)"
      | default
      = 1,

    submodules
      | Bool
      | doc "Recurse into submodules during clone"
      | default
      = false,
  },
}

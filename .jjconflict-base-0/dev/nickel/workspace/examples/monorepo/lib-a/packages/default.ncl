# lib-a default package
#
# Auto-discovered from lib-a/packages/default.ncl.
# Because this is named "default" inside a subworkspace named "lib-a",
# it appears in the flake outputs as packages.<system>.lib-a
# (the subworkspace directory name replaces "default").
{
  description = "lib-a core library",
  build-system = "generic",
}

# External tool default package
#
# Auto-discovered from external-tool/packages/default.ncl.
# Because this is named "default" inside a subworkspace named "external-tool",
# it appears in the flake outputs as packages.<system>.external-tool
# (the subworkspace directory name replaces "default").
{
  description = "External tool main binary",
  build-system = "generic",
}

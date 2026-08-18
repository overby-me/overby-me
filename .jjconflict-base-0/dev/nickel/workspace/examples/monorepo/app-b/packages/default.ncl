# app-b default package
#
# Auto-discovered from app-b/packages/default.ncl.
# Because this is named "default" inside a subworkspace named "app-b",
# it appears in the flake outputs as packages.<system>.app-b
# (the subworkspace directory name replaces "default").
{
  description = "app-b main application",
  build-system = "generic",
}

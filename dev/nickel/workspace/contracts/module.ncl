# Module contracts for nix-workspace
#
# Defines contracts for NixOS modules and home-manager modules.
# These are discovered from the modules/ and home/ convention directories
# or declared explicitly in workspace.ncl.
#
# NixOS modules  → nixosModules.<name>  flake output
# Home modules   → homeModules.<name>   flake output
let { ModuleRef, NonEmptyString, .. } = import "./common.ncl" in

{
  ModuleConfig = {
    description
      | String
      | doc "Human-readable description of what this module provides"
      | optional,

    imports
      | Array ModuleRef
      | doc m%"
          Other modules this module depends on.
          Can be module names (resolved from the workspace's modules/ directory)
          or paths to .nix files.
        "%
      | default
      = [],

    options-namespace
      | String
      | doc m%"
          The attribute path under which this module's options are declared.
          For example, "services.my-service" means options are at
          config.services.my-service in the NixOS configuration.
        "%
      | optional,

    platforms
      | Array String
      | doc m%"
          Systems this module is compatible with.
          If not set, the module is available on all workspace systems.
        "%
      | optional,

    path
      | String
      | doc m%"
          Explicit path to the .nix module file, relative to the workspace root.
          When auto-discovered, this is set automatically from the convention directory.
        "%
      | optional,

    extra-config
      | { _ : Dyn }
      | doc m%"
          Escape hatch: additional configuration passed through to the module.
          This is merged into the module's config output. Use sparingly.
        "%
      | default
      = {},
  },

  HomeConfig = {
    description
      | String
      | doc "Human-readable description of what this home-manager module provides"
      | optional,

    imports
      | Array ModuleRef
      | doc m%"
          Other home-manager modules this module depends on.
          Can be module names (resolved from the workspace's home/ directory)
          or paths to .nix files.
        "%
      | default
      = [],

    options-namespace
      | String
      | doc m%"
          The attribute path under which this module's options are declared
          within the home-manager configuration tree.
          For example, "programs.my-tool" means options are at
          home.programs.my-tool.
        "%
      | optional,

    platforms
      | Array String
      | doc m%"
          Systems this home-manager module is compatible with.
          If not set, the module is available on all workspace systems.
        "%
      | optional,

    path
      | String
      | doc m%"
          Explicit path to the .nix home-manager module file, relative to the workspace root.
          When auto-discovered, this is set automatically from the convention directory.
        "%
      | optional,

    state-version
      | String
      | doc m%"
          Home-manager state version for this module.
          Usually set at the user level in MachineConfig rather than per-module,
          but can be overridden here if needed.
        "%
      | optional,

    extra-config
      | { _ : Dyn }
      | doc m%"
          Escape hatch: additional home-manager configuration passed through
          to the module. This is merged into the module's config output.
        "%
      | default
      = {},
  },
}

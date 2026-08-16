# Shell contracts for nix-workspace
# Defines the ShellConfig contract for development shell definitions.
let { System, .. } = import "./common.ncl" in

{
  ShellConfig = {
    packages
      | Array String
      | doc "Packages to include in the shell environment"
      | default
      = [],

    env
      | { _ : String }
      | doc "Environment variables to set in the shell"
      | default
      = {},

    shell-hook
      | String
      | doc "Bash script to run when entering the shell"
      | default
      = "",

    tools
      | { _ : String }
      | doc m%"
          Tool specifications as name-version pairs.
          The key is the package attribute name, the value is a version constraint or empty string for latest.
        "%
      | default
      = {},

    systems
      | Array System
      | doc "Override systems for this shell (defaults to workspace systems)"
      | optional,

    inputs-from
      | Array String
      | doc "Other packages whose build inputs should be included"
      | default
      = [],
  },
}

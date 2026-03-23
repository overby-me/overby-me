# Tangled workflow contract
#
# Defines the schema for `.tangled/workflows/*.yml` files.
# Each YAML file in that directory is a separate workflow within a pipeline.
# Workflows within a pipeline execute in parallel; steps within a workflow
# execute serially.
#
# Reference: https://docs.tangled.org
let { Engine, .. } = import "./engine.ncl" in
let { Constraint, .. } = import "./trigger.ncl" in
let { CloneConfig, .. } = import "./clone.ncl" in
let { Dependencies, .. } = import "./dependencies.ncl" in
let { StepConfig, .. } = import "./step.ncl" in

{
  WorkflowConfig = {
    when
      | Array Constraint
      | doc "Trigger constraints that determine when this workflow runs",

    engine
      | Engine
      | doc "Execution engine for this workflow",

    clone
      | CloneConfig
      | doc "Repository clone behavior"
      | default
      = {},

    dependencies
      | Dependencies
      | doc m%"
        Packages to install, keyed by Nix registry.
        Keys follow nix registry syntax (e.g. "nixpkgs", "nixpkgs/nixpkgs-unstable",
        "git+https://tangled.org/@example.com/my_pkg").
        Values are lists of package attribute names.
      "%
      | default
      = {},

    env
      | { _ : String }
      | doc m%"
        Global environment variables injected into every step (shorthand).
        Takes precedence over "environment" when both are set.
      "%
      | default
      = {},

    environment
      | { _ : String }
      | doc m%"
        Global environment variables injected into every step.
        "env" is the preferred shorthand; both are accepted for compatibility.
      "%
      | default
      = {},

    steps
      | Array StepConfig
      | doc "Ordered list of steps to execute serially",
  },
}

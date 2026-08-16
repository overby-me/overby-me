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
        Packages to install.
        microvm engine: a flat list of package names, e.g. [ "go", "gcc" ]
        (NixOS images only). Bare names resolve from nixpkgs; flake references
        such as "github:nixos/nixpkgs#hello" are also accepted.
        nixery/nix engines: a map keyed by Nix registry (e.g. "nixpkgs",
        "nixpkgs/nixpkgs-unstable", "git+https://tangled.org/@example.com/my_pkg")
        to lists of package attribute names.
      "%
      | default
      = {},

    registry
      | { _ : String }
      | doc m%"
        microvm only. Remaps flake references, the same way `nix registry` does,
        letting you pin or alias the flakes used by "dependencies"
        (e.g. nixpkgs = "github:nixos/nixpkgs/nixos-unstable").
      "%
      | optional,

    caches
      | { _ : String }
      | doc m%"
        microvm only. Map of Nix binary cache URL to its trusted public key,
        used to substitute dependencies instead of building them.
      "%
      | optional,

    services
      | doc m%"
        microvm only. Passed straight through to NixOS — anything you could
        write under `services.*` in a NixOS configuration (e.g. a PostgreSQL
        database) you can write here.
      "%
      | optional,

    virtualisation
      | doc m%"
        microvm only. Passed straight through to NixOS — anything you could
        write under `virtualisation.*` (e.g. virtualisation.docker.enable = true
        to run Docker inside the VM).
      "%
      | optional,

    image
      | String
      | doc m%"
        microvm only. Guest image to boot. "nixos" (the default) enables the
        full workflow-level configuration above; non-NixOS images such as
        "alpine" support only plain package installation.
      "%
      | optional,

    environment
      | { _ : String }
      | doc m%"
        Global environment variables injected into every step. Omitted from the
        generated YAML when empty (tangled rejects an unknown "env" field and
        needs no explicit empty map here).
      "%
      | optional,

    steps
      | Array StepConfig
      | doc "Ordered list of steps to execute serially",
  },
}

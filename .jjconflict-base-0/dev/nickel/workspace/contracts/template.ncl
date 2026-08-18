# Template contracts for nix-workspace
#
# Defines the TemplateConfig contract for flake template definitions.
# Templates are discovered from the templates/ convention directory
# or declared explicitly in workspace.ncl.
#
# Each template config maps to a templates.<name> flake output.
# Templates are used by `nix flake init -t` to scaffold new projects
# from a template directory.
let { NonEmptyString, RelativePath, .. } = import "./common.ncl" in

{
  TemplateConfig = {
    description
      | NonEmptyString
      | doc m%"
          Human-readable description of what this template provides.
          This is displayed by `nix flake show` and `nix flake init --list`.
          Required — templates without descriptions are not useful.
        "%,

    path
      | String
      | doc m%"
          Path to the template directory, relative to the workspace root.
          This directory is copied verbatim when a user runs
          `nix flake init -t <flake>#<name>`.
          When auto-discovered from templates/, this is set automatically
          to the directory containing the template's .ncl config.
        "%
      | optional,

    welcome-text
      | String
      | doc m%"
          Text displayed to the user after the template is instantiated.
          Use this to provide next-step instructions, e.g.:
            "Run 'nix develop' to enter the dev shell."
        "%
      | optional,

    tags
      | Array String
      | doc m%"
          Tags for categorizing the template (e.g. "rust", "web", "minimal").
          Informational metadata — not used by Nix, but useful for tooling
          and documentation.
        "%
      | default
      = [],

    extra-config
      | { _ : Dyn }
      | doc m%"
          Escape hatch: additional configuration passed through to the
          template builder. Use sparingly.
        "%
      | default
      = {},
  },
}

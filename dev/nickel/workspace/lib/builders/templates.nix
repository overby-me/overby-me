# Template builder for nix-workspace
#
# Converts validated TemplateConfig records into flake template outputs.
# Templates are discovered from the templates/ convention directory
# or declared explicitly in workspace.ncl.
#
# Each template config maps to a templates.<name> flake output.
# Templates are used by `nix flake init -t <flake>#<name>` to scaffold
# new projects from a template directory.
#
# Flake template output shape (per Nix specification):
#   templates.<name> = {
#     description = "Human-readable description";
#     path = /nix/store/...-template-dir;
#     welcomeText = "Optional post-init message";  # optional
#   };
#
# Input shape (from evaluated workspace.ncl):
#   {
#     description = "Minimal Rust workspace";
#     path = "./templates/rust-minimal";
#     welcome-text = "Run 'nix develop' to enter the dev shell.";
#     tags = ["rust" "minimal"];
#     extra-config = {};
#   }
#
{lib}: let
  # Build a single flake template output from a TemplateConfig.
  #
  # The template output is a simple attribute set with `description`,
  # `path`, and optionally `welcomeText`. The `path` must point to
  # a directory that will be copied when a user runs `nix flake init`.
  #
  # Type: Path -> String -> AttrSet -> AttrSet
  #
  # Arguments:
  #   workspaceRoot   — Path to the workspace root directory
  #   name            — Template name (e.g. "rust-minimal")
  #   templateConfig  — The evaluated TemplateConfig from Nickel
  #
  # Returns: A flake template attribute set { description, path, welcomeText? }
  #
  buildTemplate = workspaceRoot: name: templateConfig: let
    description =
      templateConfig.description
        or "nix-workspace template: ${name}";

    hasPath = templateConfig ? path;

    resolvedPath =
      if hasPath
      then
        if lib.hasPrefix "./" templateConfig.path || lib.hasPrefix "../" templateConfig.path
        then workspaceRoot + "/${templateConfig.path}"
        else if lib.hasPrefix "/" templateConfig.path
        then /. + templateConfig.path
        else workspaceRoot + "/${templateConfig.path}"
      else
        # Default: assume the template directory matches the template name
        # under the templates/ convention directory.
        workspaceRoot + "/templates/${name}";

    hasWelcomeText =
      templateConfig ? welcome-text
      && templateConfig.welcome-text != "";

    baseOutput = {
      inherit description;
      path = resolvedPath;
    };

    withWelcomeText =
      if hasWelcomeText
      then baseOutput // {welcomeText = templateConfig.welcome-text;}
      else baseOutput;
  in
    withWelcomeText
    // (templateConfig.extra-config or {});

  # Build all templates from the workspace config.
  #
  # Type: AttrSet -> AttrSet
  #
  # Arguments:
  #   workspaceRoot    — Path to the workspace root
  #   templateConfigs  — { name = TemplateConfig; ... } from workspace evaluation
  #   discoveredPaths  — { name = /path/to/template-dir; ... } from auto-discovery
  #
  # Note on discovery: Template discovery works slightly differently from
  # other convention types. The templates/ directory contains subdirectories
  # (not .ncl files) that are the template content. If a template has a
  # companion .ncl config, that provides metadata (description, welcome-text).
  # If no .ncl config exists, a minimal template output is generated from
  # the directory alone.
  #
  # Returns:
  #   { name = templateOutput; ... } suitable for the templates flake output
  #
  buildAllTemplates = {
    workspaceRoot,
    templateConfigs,
    discoveredPaths ? {},
  }: let
    # For discovered templates without explicit config, create minimal configs
    # using the discovered path. The path from discovery points to the .ncl file,
    # but for templates we want the directory. We strip the .ncl filename and
    # use the convention directory.
    discoveredConfigs =
      lib.mapAttrs (
        name: _path: {
          description = "Template: ${name}";
        }
      )
      discoveredPaths;

    effectiveConfigs = discoveredConfigs // templateConfigs;
  in
    lib.mapAttrs (
      name: cfg:
        buildTemplate workspaceRoot name cfg
    )
    effectiveConfigs;

  # Discover template directories (subdirectories of the templates/ convention dir).
  #
  # Unlike other conventions that discover .ncl files, templates discover
  # directories. A template is a subdirectory of templates/ that can be
  # copied by `nix flake init`.
  #
  # Type: Path -> AttrSet
  #
  # Arguments:
  #   workspaceRoot — Path to the workspace root
  #
  # Returns:
  #   { name = /path/to/templates/name; ... }
  #
  discoverTemplateDirs = workspaceRoot: let
    templatesDir = workspaceRoot + "/templates";
  in
    if builtins.pathExists templatesDir
    then let
      entries = builtins.readDir templatesDir;
      dirs =
        lib.filterAttrs (
          _name: type:
            type == "directory" || type == "symlink"
        )
        entries;
    in
      lib.mapAttrs (
        name: _: templatesDir + "/${name}"
      )
      dirs
    else {};
in {
  inherit
    buildTemplate
    buildAllTemplates
    discoverTemplateDirs
    ;
}

# Pure devshell builder.
#
# Evaluates a list of devshell modules with `lib.evalModules` and realises the
# merged config into a `pkgs.mkShell` derivation. Replaces the devenv-based
# shell construction: each module contributes `packages`, `env`, `shellHook`
# (concatenated) and `inputsFrom`, and the working directory is discovered at
# runtime by the module shellHooks (git-hooks installs relative to the git
# root), so no working-directory flake input is needed.
#
# Usage:
#   mkDevShell = import ./mkDevShell.nix { inherit lib inputs; };
#   mkDevShell pkgs [ ./modules/common.nix ./modules/git-hooks.nix ... ]
{
  lib,
  inputs,
}: let
  inherit (lib) mkOption types;

  baseModule = {pkgs, ...}: {
    options = {
      name = mkOption {
        type = types.str;
        default = "dev-shell";
        description = "Derivation name of the shell.";
      };
      packages = mkOption {
        type = types.listOf types.package;
        default = [];
        description = "Packages placed on PATH in the shell.";
      };
      env = mkOption {
        type = types.attrsOf (types.oneOf [types.str types.path types.int]);
        default = {};
        description = "Environment variables exported into the shell.";
      };
      shellHook = mkOption {
        type = types.lines;
        default = "";
        description = "Shell script run on shell entry (modules concatenate).";
      };
      inputsFrom = mkOption {
        type = types.listOf types.unspecified;
        default = [];
        description = "Derivations whose build inputs are pulled into the shell.";
      };
      stdenv = mkOption {
        type = types.unspecified;
        default = pkgs.stdenv;
        description = "stdenv used to construct the shell.";
      };
    };
  };
in
  pkgs: modules: let
    eval = lib.evalModules {
      specialArgs = {
        inherit pkgs inputs lib;
        src = inputs.self;
      };
      modules = [baseModule] ++ modules;
    };
    cfg = eval.config;
  in
    pkgs.mkShell.override {inherit (cfg) stdenv;}
    (cfg.env
      // {
        inherit (cfg) name packages inputsFrom shellHook;
      })

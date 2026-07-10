{
  config,
  lib,
  inputs,
  ...
}: let
  inherit (lib) mkOption mkMerge concatMap mapAttrs mapAttrsToList removeAttrs;
  inherit (lib.types) lazyAttrsOf unspecified;

  mkDevenvShell = pkgs: cfg:
    inputs.devenv.lib.mkShell {
      inherit inputs pkgs lib;
      modules = [
        {_module.args.src = inputs.self.outPath;}
        cfg
      ];
    };

  # Resolve a flakelight devShell config value into a derivation.
  # This replicates flakelight's internal genDevShell logic:
  # - If overrideShell is set (e.g. devenv shells coerced as packages), use it directly
  # - Otherwise unwrap optFunctionTo values by calling them with pkgs, then mkShell
  resolveDevShell = pkgs: shellFn: let
    cfg = shellFn pkgs;
  in
    if cfg ? overrideShell && cfg.overrideShell != null
    then cfg.overrideShell
    else let
      # optFunctionTo values are functors that need to be called with pkgs
      # hardeningDisable is a plain list, but laziness means we never force
      # cfg'.hardeningDisable since we take it from cfg instead
      cfg' = mapAttrs (_: v: v pkgs) cfg;
    in
      pkgs.mkShell.override {inherit (cfg') stdenv;}
      (cfg'.env
        // {
          inherit (cfg') inputsFrom packages shellHook;
          inherit (cfg) hardeningDisable;
        });
in {
  options = {
    devenvConfigurations = mkOption {
      type = lazyAttrsOf unspecified;
      default = {};
      description = "Devenv configurations to export as devShells";
    };

    devenvConfiguration = mkOption {
      type = unspecified;
      default = null;
      description = "Devenv configuration to export as the default devShell";
    };
  };

  config = mkMerge [
    {
      devShells =
        mapAttrs (_: cfg: pkgs: mkDevenvShell pkgs cfg)
        config.devenvConfigurations;
    }

    (lib.mkIf (config.devenvConfiguration != null) {
      devShells.default = pkgs: let
        devenvShell = mkDevenvShell pkgs config.devenvConfiguration;
        otherShells =
          mapAttrsToList
          (_: shellFn: resolveDevShell pkgs shellFn)
          (removeAttrs config.devShells ["default"]);
      in
        # Use overrideAttrs to preserve devenv environment variables
        # (DEVENV_ROOT, DEVENV_STATE, etc.) and shell hooks.
        # A plain mkShell wrapper with inputsFrom only propagates build
        # inputs, losing the env vars the devenv shellHook depends on.
        devenvShell.overrideAttrs (old: {
          buildInputs =
            (old.buildInputs or [])
            ++ concatMap (s: s.buildInputs or []) otherShells;
          nativeBuildInputs =
            (old.nativeBuildInputs or [])
            ++ concatMap (s: s.nativeBuildInputs or []) otherShells;
        });
    })

    {nixDirPathAttrs = ["devenvConfigurations" "devenvConfiguration"];}
  ];
}

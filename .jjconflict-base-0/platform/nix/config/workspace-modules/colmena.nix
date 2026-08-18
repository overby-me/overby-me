{
  config,
  inputs,
  lib,
  ...
}: let
  isNixos = x: x ? config.system.build.toplevel;

  standardConfigs =
    lib.filterAttrs (_: cfg: !(isNixos cfg)) config.nixosConfigurations;

  # Colmena's eval.nix accesses `pkgs.system` on the nixpkgs sets we provide
  # via meta.nixpkgs / meta.nodeNixpkgs.  In recent nixpkgs that attribute is
  # a deprecated alias (warnAlias) that emits:
  #   evaluation warning: 'system' has been renamed to/replaced by
  #   'stdenv.hostPlatform.system'
  # Shadow the alias with the plain value so colmena's `inherit (npkgs) system;`
  # no longer triggers the warning.
  suppressSystemWarning = pkgs:
    pkgs // {inherit (pkgs.stdenv.hostPlatform) system;};
in {
  outputs.colmena =
    {
      meta = {
        nixpkgs = suppressSystemWarning (import inputs.nixpkgs {
          system = "x86_64-linux";
          config.allowUnfree = true;
        });
        nodeNixpkgs = lib.mapAttrs (_: cfg:
          suppressSystemWarning (import inputs.nixpkgs {
            inherit (cfg) system;
            config.allowUnfree = true;
          }))
        standardConfigs;
        nodeSpecialArgs = lib.mapAttrs (name: cfg:
          {
            inherit inputs;
            hostname = name;
          }
          // (cfg.specialArgs or {}))
        standardConfigs;
      };
    }
    // lib.mapAttrs (name: cfg: {
      deployment = {
        targetHost = "${name}.overby.me";
        targetUser = "overby.me";
      };
      imports =
        [
          config.propagationModule
          ({flake, ...}: {_module.args = {inherit (flake) inputs';};})
        ]
        ++ (cfg.modules or []);
    })
    standardConfigs;
}

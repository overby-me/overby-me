# Machine builder for nix-workspace
#
# Converts validated MachineConfig records into NixOS system configurations.
# Each machine config (from workspace.ncl or discovered machines/*.ncl files)
# is mapped to a nixosConfigurations.<name> flake output using
# nixpkgs.lib.nixosSystem.
#
# Input shape (from evaluated workspace.ncl):
#   {
#     system = "x86_64-linux";
#     state-version = "25.05";
#     modules = ["desktop" "development"];
#     host-name = "gravitas";
#     special-args = {};
#     users = { alice = { home-modules = ["shell" "editor"]; }; };
#     boot-loader = "systemd-boot";
#     file-systems = { "/" = { device = "/dev/sda1"; fs-type = "ext4"; }; };
#     networking = { firewall = { enable = true; }; };
#     time-zone = "Europe/Copenhagen";
#     locale = "en_US.UTF-8";
#     extra-config = {};
#   }
#
{lib}: let
  # Build a single NixOS configuration from a MachineConfig.
  #
  # Type: Nixpkgs -> Path -> String -> AttrSet -> AttrSet -> AttrSet -> AttrSet -> Derivation
  #
  # Arguments:
  #   nixpkgs          — The nixpkgs flake input (for nixpkgs.lib.nixosSystem)
  #   workspaceRoot    — Path to the workspace root directory
  #   name             — Machine name (becomes the hostname if not overridden)
  #   machineConfig    — The evaluated MachineConfig from Nickel
  #   workspaceModules — { name = path; } of NixOS modules from the workspace
  #   homeModules      — { name = path; } of home-manager modules from the workspace
  #   extraInputs      — Additional flake inputs to pass as specialArgs
  #
  buildMachine = {
    nixpkgs,
    workspaceRoot,
    name,
    machineConfig,
    workspaceModules ? {},
    homeModules ? {},
    extraInputs ? {},
  }: let
    inherit (machineConfig) system;
    stateVersion = machineConfig.state-version or "25.05";
    hostName = machineConfig.host-name or name;
    specialArgs =
      (machineConfig.special-args or {})
      // {
        inherit workspaceRoot;
        flakeInputs = extraInputs;
      };

    # ── Boot loader module ──────────────────────────────────────
    bootLoaderModule = let
      bootLoader = machineConfig.boot-loader or "systemd-boot";
    in
      if bootLoader == "systemd-boot"
      then {
        boot.loader.systemd-boot.enable = true;
        boot.loader.efi.canTouchEfiVariables = true;
      }
      else if bootLoader == "grub"
      then {
        boot.loader.grub.enable = true;
      }
      else
        # "none" — user manages boot loader via modules
        {};

    # ── File systems module ─────────────────────────────────────
    fileSystemsModule = let
      fsCfg = machineConfig.file-systems or {};
    in {
      fileSystems =
        lib.mapAttrs (
          _mountPoint: cfg: {
            inherit (cfg) device;
            fsType = cfg.fs-type or "ext4";
            options = cfg.options or [];
            neededForBoot = cfg.needed-for-boot or false;
          }
        )
        fsCfg;
    };

    # ── Networking module ───────────────────────────────────────
    networkingModule = let
      netCfg = machineConfig.networking or {};
      fwCfg = netCfg.firewall or {};
      ifCfgs = netCfg.interfaces or {};
    in {
      networking = {
        inherit hostName;
        useDHCP = lib.mkDefault (netCfg.use-dhcp or true);

        firewall = {
          enable = fwCfg.enable or true;
          allowedTCPPorts = fwCfg.allowed-tcp-ports or [];
          allowedUDPPorts = fwCfg.allowed-udp-ports or [];
        };

        interfaces =
          lib.mapAttrs (
            _ifName: ifCfg: {
              useDHCP = ifCfg.use-dhcp or true;
              ipv4.addresses = map (
                addr: let
                  parts = lib.splitString "/" addr;
                in {
                  address = builtins.head parts;
                  prefixLength =
                    if builtins.length parts > 1
                    then lib.toInt (builtins.elemAt parts 1)
                    else 24;
                }
              ) (ifCfg.ip-addresses or []);
            }
          )
          ifCfgs;

        wireless.enable = netCfg.wireless or false;
      };
    };

    # ── Locale / timezone module ────────────────────────────────
    localeModule =
      {}
      // (lib.optionalAttrs (machineConfig ? time-zone) {
        time.timeZone = machineConfig.time-zone;
      })
      // (lib.optionalAttrs (machineConfig ? locale) {
        i18n.defaultLocale = machineConfig.locale;
      });

    # ── Users module ────────────────────────────────────────────
    usersModule = let
      usersCfg = machineConfig.users or {};
    in {
      users.users =
        lib.mapAttrs (
          _userName: userCfg:
            {
              isNormalUser = userCfg.is-normal-user or true;
              extraGroups = userCfg.extra-groups or [];
            }
            // (lib.optionalAttrs (userCfg ? shell) {
              inherit (userCfg) shell;
            })
        )
        usersCfg;
    };

    # ── Resolve workspace module references ─────────────────────
    #
    # Module references in the machine config can be:
    #   - A name matching a key in workspaceModules (e.g. "desktop")
    #   - A relative path to a .nix file (e.g. "./modules/custom.nix")
    #   - An absolute path to a .nix file
    #
    resolveModuleRef = ref:
      if builtins.hasAttr ref workspaceModules
      then workspaceModules.${ref}
      else if lib.hasPrefix "./" ref || lib.hasPrefix "../" ref
      then workspaceRoot + "/${ref}"
      else if lib.hasPrefix "/" ref
      then /. + ref
      else
        throw ''
          nix-workspace: machine '${name}' references module '${ref}' which was not found.
          Available workspace modules: ${builtins.concatStringsSep ", " (builtins.attrNames workspaceModules)}
          Hint: module references can be a workspace module name, a relative path (./path), or an absolute path.
        '';

    resolvedModuleRefs =
      map resolveModuleRef (machineConfig.modules or []);

    # ── State version module ────────────────────────────────────
    stateVersionModule = {
      system.stateVersion = stateVersion;
    };

    # ── Extra config module ─────────────────────────────────────
    extraConfigModule = machineConfig.extra-config or {};

    # ── Home-manager integration ────────────────────────────────
    #
    # If any user has home-manager enabled, we need to include the
    # home-manager NixOS module and configure per-user home configs.
    #
    hmUsers = lib.filterAttrs (
      _: userCfg: userCfg.home-manager or true
    ) (machineConfig.users or {});

    hasHomeManager = hmUsers != {};

    homeManagerModule =
      if hasHomeManager
      then let
        resolveHomeModuleRef = ref:
          if builtins.hasAttr ref homeModules
          then homeModules.${ref}
          else if lib.hasPrefix "./" ref || lib.hasPrefix "../" ref
          then workspaceRoot + "/${ref}"
          else if lib.hasPrefix "/" ref
          then /. + ref
          else
            throw ''
              nix-workspace: home-manager module '${ref}' not found.
              Available home modules: ${builtins.concatStringsSep ", " (builtins.attrNames homeModules)}
            '';
      in {
        home-manager = {
          useGlobalPkgs = true;
          useUserPackages = true;
          users =
            lib.mapAttrs (
              _userName: userCfg: {...}: {
                imports =
                  map resolveHomeModuleRef (userCfg.home-modules or []);

                home.stateVersion = stateVersion;
              }
            )
            hmUsers;
        };
      }
      else {};

    # ── Assemble all modules ────────────────────────────────────
    allModules =
      [
        bootLoaderModule
        fileSystemsModule
        networkingModule
        localeModule
        usersModule
        stateVersionModule
        extraConfigModule
      ]
      ++ resolvedModuleRefs
      ++ (lib.optional hasHomeManager homeManagerModule);
  in
    nixpkgs.lib.nixosSystem {
      inherit system specialArgs;
      modules = allModules;
    };

  # Build all machine configurations from the workspace config.
  #
  # Type: AttrSet -> AttrSet
  #
  # Arguments:
  #   nixpkgs          — The nixpkgs flake input
  #   workspaceRoot    — Path to the workspace root
  #   machineConfigs   — { name = MachineConfig; ... } from workspace evaluation
  #   workspaceModules — { name = /path/to/module.nix; ... } discovered NixOS modules
  #   homeModules      — { name = /path/to/module.nix; ... } discovered home-manager modules
  #   extraInputs      — Additional flake inputs
  #
  # Returns:
  #   { name = nixosConfiguration; ... } suitable for nixosConfigurations
  #
  buildAllMachines = {
    nixpkgs,
    workspaceRoot,
    machineConfigs,
    workspaceModules ? {},
    homeModules ? {},
    extraInputs ? {},
  }:
    lib.mapAttrs (
      name: machineConfig:
        buildMachine {
          inherit nixpkgs workspaceRoot name machineConfig workspaceModules homeModules extraInputs;
        }
    )
    machineConfigs;
in {
  inherit buildMachine buildAllMachines;
}

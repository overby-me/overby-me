# Kanata keyboard remapping, ported to system-manager.
#
# This machine runs kanata on NixOS via `services.kanata` (see
# `nixos/modules/services/kanata.nix`). That NixOS module can't be reused under
# system-manager: it sets `hardware.uinput.enable` and references
# `config.users.groups`, neither of which exists in system-manager's module set
# (system-manager doesn't manage kernel modules, udev, or users/groups).
#
# So this replicates the essential output of the NixOS `services.kanata` module
# — the generated config file plus a hardened systemd service — but adapted for
# a non-NixOS host:
#
#   - No `hardware.uinput.enable`: the host is expected to already have
#     `/dev/uinput` (owned `root:input`) with the `uinput` kernel module
#     available, so no module-loading/udev management is needed.
#   - `SupplementaryGroups = ["input"]` only: NixOS uses both `input` and
#     `uinput` groups, but a stock Ubuntu host has no `uinput` group; the
#     `input` group grants the rw access to `/dev/uinput` that kanata needs.
#
# The keyboard config below is kept in sync with
# `nixos/modules/services/kanata.nix` (caps-lock as a nav-layer tap-hold).
{
  pkgs,
  lib,
  ...
}: let
  name = "default";
  serviceName = "kanata-${name}";

  # Mirrors services.kanata.keyboards.default config from the NixOS module.
  keyboardConfig = ''
    (defsrc
      caps
      left down up   rght
      h    j    k    l
    )

    (defalias
      nav (tap-hold 200 200 caps (layer-while-held nav))
    )

    (deflayer base
      @nav
      XX   XX   XX   XX
      h    j    k    l
    )

    (deflayer nav
      _
      XX   XX   XX   XX
      left down up   rght
    )
  '';

  # Same `defcfg` wrapper the NixOS module generates: no `linux-dev` (empty
  # devices means kanata auto-detects all keyboards), continue if none found,
  # and `process-unmapped-keys yes` (from the NixOS extraDefCfg).
  configFile = pkgs.writeTextFile {
    name = "${serviceName}-config.kbd";
    text = ''
      (defcfg
        process-unmapped-keys yes
        linux-continue-if-no-devs-found yes)

      ${keyboardConfig}
    '';
    checkPhase = ''
      ${lib.getExe pkgs.kanata} --cfg "$target" --check --debug
    '';
  };
in {
  systemd.services.${serviceName} = {
    description = "kanata keyboard remapping (${name})";
    # `multi-user.target` is remapped by system-manager to `system-manager.target`.
    wantedBy = ["multi-user.target"];

    serviceConfig = {
      Type = "notify";
      ExecStart = ''
        ${lib.getExe pkgs.kanata} \
          --cfg ${configFile} \
          --symlink-path ''${RUNTIME_DIRECTORY}/${name}
      '';

      DynamicUser = true;
      RuntimeDirectory = serviceName;
      # `input` group grants rw on /dev/uinput on a stock host (no `uinput` group).
      SupplementaryGroups = ["input"];

      # Hardening, mirroring the upstream NixOS kanata module.
      DeviceAllow = [
        "/dev/uinput rw"
        "char-input r"
      ];
      CapabilityBoundingSet = [""];
      DevicePolicy = "closed";
      IPAddressDeny = ["any"];
      LockPersonality = true;
      MemoryDenyWriteExecute = true;
      PrivateNetwork = true;
      PrivateUsers = true;
      ProcSubset = "pid";
      ProtectClock = true;
      ProtectControlGroups = true;
      ProtectHome = true;
      ProtectHostname = true;
      ProtectKernelLogs = true;
      ProtectKernelModules = true;
      ProtectKernelTunables = true;
      ProtectProc = "invisible";
      RestrictAddressFamilies = ["AF_UNIX"];
      RestrictNamespaces = true;
      RestrictRealtime = true;
      SystemCallArchitectures = ["native"];
      SystemCallFilter = [
        "@system-service"
        "~@privileged"
        "~@resources"
      ];
      UMask = "0077";
    };
  };
}

# Kanata keyboard remapping for system-manager, kept in sync with the NixOS
# module at platform/nix/config/nixos-modules/services/kanata.nix.
#
# That module cannot be reused here: it sets `hardware.uinput.enable` and reads
# `config.users.groups`, and system-manager manages neither kernel modules nor
# users. So this reproduces its output - the config file and a hardened service
# - with two deviations for a non-NixOS host:
#
#   - No `hardware.uinput.enable`: the host is expected to have /dev/uinput
#     (root:input) and the module available already.
#   - `SupplementaryGroups = ["input"]` only, because a stock Ubuntu host has
#     no `uinput` group and `input` already grants rw on /dev/uinput.
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

  # The wrapper the NixOS module generates. No `linux-dev`, so kanata
  # auto-detects every keyboard.
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

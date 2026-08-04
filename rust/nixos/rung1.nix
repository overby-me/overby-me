# Rung 1 (docs/ROADMAP.md "Ship one increment"): run individual rust systemd
# components under the STOCK C systemd manager, on a real machine, one at a time.
#
# PID 1, the systemd package, and every other unit stay stock C; enabling a
# component here swaps only that component's binary in via a service override, so
# a regression fails in isolation and a NixOS generation rollback fully reverts
# it -- exactly the incremental-adoption path README design principle 5 describes.
# The rust binaries carry their own resolved helper paths (setfacl for tmpfiles,
# useradd/groupadd/chage for sysusers), so they work from the boot services'
# minimal $PATH.
#
# Usage on a real machine:
#   imports = [ /path/to/rust/nixos/rung1.nix ];
#   services.rustSystemdRung1.tmpfiles.enable = true;
#
# The tmpfiles toggle is validated in-VM through this module by
# rust/nixos/rung1-tmpfiles-test.nix. The sysusers toggle runs the same rust
# systemd-sysusers that rung1-sysusers-test.nix proves works under C PID 1 (that
# test drives a scoped config; this toggle runs the full sysusers.d idempotently).
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.rustSystemdRung1;
  rustSystemd = pkgs.rust-systemd;
in {
  options.services.rustSystemdRung1 = {
    tmpfiles.enable =
      lib.mkEnableOption "running the rust systemd-tmpfiles for boot-time tmpfiles setup under C PID 1";
    sysusers.enable =
      lib.mkEnableOption "running the rust systemd-sysusers as a dedicated boot oneshot under C PID 1";
    timesyncd.enable =
      lib.mkEnableOption "running the rust systemd-timesyncd daemon under C PID 1 (assumes services.timesyncd.enable)";
  };

  config = lib.mkMerge [
    (lib.mkIf cfg.tmpfiles.enable {
      # Redirect the boot systemd-tmpfiles-setup service to the rust binary.
      systemd.services.systemd-tmpfiles-setup.serviceConfig.ExecStart = lib.mkForce [
        ""
        "${rustSystemd}/bin/systemd-tmpfiles --create --remove --boot --exclude-prefix=/dev"
      ];
    })
    (lib.mkIf cfg.sysusers.enable {
      # A dedicated oneshot runs rust systemd-sysusers over the full sysusers.d
      # set. It is idempotent (existing users/groups are a no-op), so it is safe
      # alongside the C-created system users; do NOT override the boot
      # systemd-sysusers.service itself (that starves early name lookups).
      systemd.services.rust-sysusers = {
        description = "rust systemd-sysusers under C PID 1 (rung 1)";
        wantedBy = ["multi-user.target"];
        after = ["systemd-sysusers.service"];
        serviceConfig = {
          Type = "oneshot";
          RemainAfterExit = true;
          ExecStart = "${rustSystemd}/bin/systemd-sysusers";
        };
      };
    })
    (lib.mkIf cfg.timesyncd.enable {
      # Redirect the timesyncd daemon (Type=notify) to the rust binary. Not
      # boot-critical, so a failure fails only this service. Requires the
      # timesyncd service to be enabled (the NixOS default).
      systemd.services.systemd-timesyncd.serviceConfig.ExecStart = lib.mkForce [
        ""
        "${rustSystemd}/bin/systemd-timesyncd"
      ];
    })
  ];
}

# Fairphone 5 modem module: Qualcomm QDM5577 cellular modem userspace services.
#
# Services configured:
#   - ModemManager: High-level modem management
#   - pd-mapper: Protection Domain Mapper (routes messages between subsystems)
#   - rmtfs: Remote Filesystem Service (provides calibration partition access)
#   - tqftpserv: TFTP server over QRTR (provides firmware to modem)
#   - msm-modem-uim-selection: SIM card slot selection
#
# Note: qrtr-ns is not needed as the kernel provides QRTR namespace functionality.
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.nixos-fairphone-fp5.modem;
in {
  options.nixos-fairphone-fp5.modem = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Enable Qualcomm modem support for Fairphone 5.";
    };

    verbose = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Enable verbose logging for modem services.";
    };

    quickSuspendResume = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Enable ModemManager's --test-quick-suspend-resume for better power
        management on Qualcomm devices (recommended by PostmarketOS).
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = with pkgs; [
      libqmi
      pd-mapper
      qrtr
      rmtfs
      tqftpserv
    ];

    networking.modemmanager.enable = true;

    systemd.services = {
      # Override ModemManager to optionally add --test-quick-suspend-resume.
      ModemManager.serviceConfig.ExecStart = lib.mkForce [
        "" # Clear the default ExecStart.
        "${pkgs.modemmanager}/sbin/ModemManager${lib.optionalString cfg.quickSuspendResume " --test-quick-suspend-resume"}"
      ];

      # TFTP server over QRTR.  Provides firmware files to the modem at
      # runtime via the QRTR protocol.
      tqftpserv = {
        description = "TFTP server over QRTR";
        wantedBy = ["multi-user.target"];
        serviceConfig = {
          ExecStart = "${pkgs.tqftpserv}/bin/tqftpserv${lib.optionalString cfg.verbose " -v"}";
          Restart = "always";
          RestartSec = "1";
        };
      };

      # Protection Domain Mapper.  Routes messages between modem and DSP
      # subsystems.
      pd-mapper = {
        description = "Qualcomm Protection Domain Mapper";
        wantedBy = ["multi-user.target"];
        serviceConfig = {
          ExecStart = "${pkgs.pd-mapper}/bin/pd-mapper${lib.optionalString cfg.verbose " -v"}";
          Restart = "always";
          RestartSec = "1";
        };
      };

      # Remote Filesystem Service.  Provides access to the modem's calibration
      # partitions.  Uses -P flag to access raw EFS partitions (modemst1,
      # modemst2, fsg, fsc) from /dev/disk/by-partlabel/ instead of files.
      rmtfs = {
        description = "Qualcomm Remote Filesystem Service";
        wantedBy = ["multi-user.target"];
        serviceConfig = {
          ExecStart = "${pkgs.rmtfs}/bin/rmtfs -r -P -s${lib.optionalString cfg.verbose " -v"}";
          Restart = "always";
          RestartSec = "1";
        };
      };

      # SIM card slot selection.  Runs before ModemManager and configures
      # which SIM slot to use.  Automatically selects the first present SIM.
      msm-modem-uim-selection = {
        description = "Qualcomm modem SIM card slot selection";
        before = ["ModemManager.service"];
        after = ["rmtfs.service" "pd-mapper.service" "tqftpserv.service"];
        requires = ["rmtfs.service" "pd-mapper.service" "tqftpserv.service"];
        wantedBy = ["ModemManager.service"];
        path = with pkgs; [libqmi gawk gnugrep coreutils];

        script = ''
          # Wait for modem to be ready by checking QRTR node availability.
          # Uses exponential backoff: starts at 1s, doubles each attempt up to 60s.
          attempt=1
          sleep_time=1
          max_sleep=60

          while true; do
            if qmicli --silent -pd qrtr://0 --uim-get-card-status &>/dev/null; then
              echo "Modem ready after $attempt attempt(s)"
              break
            fi

            echo "Waiting for modem (attempt $attempt, sleeping ''${sleep_time}s)..."
            sleep "$sleep_time"

            if [ "$sleep_time" -lt "$max_sleep" ]; then
              sleep_time=$((sleep_time * 2))
              if [ "$sleep_time" -gt "$max_sleep" ]; then
                sleep_time="$max_sleep"
              fi
            fi

            attempt=$((attempt + 1))
          done

          QMICLI_MODEM="qmicli --silent -pd qrtr://0"
          QMI_CARDS=$($QMICLI_MODEM --uim-get-card-status)
          if ! printf "%s" "$QMI_CARDS" | grep -Fq "Primary GW:   session doesn't exist"
          then
              $QMICLI_MODEM --uim-change-provisioning-session='activate=no,session-type=primary-gw-provisioning' > /dev/null
          fi
          FIRST_PRESENT_SLOT=$(printf "%s" "$QMI_CARDS" | grep "Card state: 'present'" -m1 -B1 | head -n1 | cut -c7-7)
          FIRST_PRESENT_AID=$(printf "%s" "$QMI_CARDS" | grep "usim (2)" -m1 -A3 | tail -n1 | awk '{print $1}')
          $QMICLI_MODEM --uim-change-provisioning-session="slot=$FIRST_PRESENT_SLOT,activate=yes,session-type=primary-gw-provisioning,aid=$FIRST_PRESENT_AID" > /dev/null
        '';
        serviceConfig = {
          Type = "oneshot";
          RemainAfterExit = true;
          Restart = "on-failure";
          RestartSec = "5";
        };
      };
    };
  };
}

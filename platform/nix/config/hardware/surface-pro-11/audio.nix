# Surface Pro 11 speakers and microphones.
#
# Nothing here patches a driver.  Linux 7.0's Denali devicetree already declares
# the sound card, the WSA and VA DAI links, both WSA8845 amplifiers and the DMIC
# pinctrl, and points remoteproc at exactly the ADSP images firmware.nix
# installs.  Four things are missing, all in userspace, and the first of them
# leaves no ALSA card at all rather than a quiet one:
#
#   the AudioReach topology, from pkgs/audio-topology-surface-pro-11.nix;
#   a UCM profile, from pkgs/alsa-ucm-conf-surface-pro-11.nix;
#   somebody to open the speaker PCM first, scripts/wsa-routing.nu;
#   a channel map PipeWire does not guess right, see speakerSink below.
#
# Derived throughout from ooaklee/linux-surface-pro-11-oe, which is where this
# was worked out on the machine; each file names the part it came from.
#
# Set expectations before switching: speakers are usable rather than good, and
# capture is thin even with the 2.4 MHz DMIC clock hardware.nix patches in.  The
# headphone jack and DisplayPort audio are not wired up at all, because mainline's
# devicetree declares no DAI link for either.
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.hardware.surfacePro11;

  # snd_card_set_id truncates the devicetree `model`,
  # "X1E80100-Microsoft-Surface-Pro-11", to 15 alphanumerics.
  card = "X1E80100Microso";

  speakerPcm = "hw:${card},1";
in {
  options.hardware.surfacePro11.audio = {
    enable = lib.mkEnableOption "audio support on the Surface Pro 11";

    speakerSink = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Whether to give PipeWire a hand-written speaker sink instead of letting
        it derive one from the UCM profile.

        The speaker PCM carries four channels and the two amplifiers sit on
        physical slots 0 and 2.  PipeWire lays a stereo stream out as
        `[ FL FR RL RR ]`, which puts the right channel on slot 1, where nothing
        is connected, and leaves the right speaker silent.  The sink this writes
        relabels the slots `[ FL RL FR RR ]` so the right channel lands on slot
        2, and takes ownership of the PCM by disabling WirePlumber's own node
        for the same device, which would otherwise fight it for an exclusive
        open.

        Turn this off if a UCM or PipeWire release ever gets the mapping right
        on its own; the card still works without it, with one speaker.
      '';
    };
  };

  config = lib.mkIf (cfg.enable && cfg.audio.enable) (lib.mkMerge [
    {
      assertions = [
        {
          assertion = cfg.firmware.enable;
          message = ''
            Audio support on the Surface Pro 11 requires firmware: the audio DSP
            runs qcadsp8380.mbn and adsp_dtb.mbn, and without remoteproc bringing
            it up there is no AudioReach to load a topology into.
          '';
        }
      ];

      hardware.firmware = [pkgs.audio-topology-surface-pro-11];

      # Not an alsa-ucm-conf overlay, which would rebuild alsa-lib and every
      # aarch64 package that links it for two configuration files.  pam_env
      # applies this to the systemd-user session too, so PipeWire sees the same
      # tree as an alsaucm run by hand to check on it.
      environment.sessionVariables.ALSA_CONFIG_UCM2 = "${pkgs.alsa-ucm-conf-surface-pro-11}/share/alsa/ucm2";

      # Every diagnostic in the files here is an alsa-utils command.
      environment.systemPackages = [pkgs.alsa-utils];

      systemd.services.surface-pro-11-wsa-routing = {
        description = "Surface Pro 11 speaker route";
        # Ordering alone is not enough, hence the polling in the script:
        # sound.target is reached before this late-probing card exists.  The
        # alsa-restore pair is here so that its register writes land before the
        # route rather than during it, on the systems that run them.
        after = ["sound.target" "alsa-restore.service" "alsa-state.service"];
        wants = ["sound.target"];
        before = ["display-manager.service"];
        wantedBy = ["multi-user.target"];
        path = [pkgs.alsa-utils pkgs.coreutils];
        serviceConfig = {
          Type = "oneshot";
          RemainAfterExit = true;
          # A backstop over the script's own budgets, not the working limit.
          # Ordering the greeter after this means every second spent failing
          # here is a second of black screen.
          TimeoutStartSec = "2min";
          ExecStart = "${lib.getExe pkgs.nushell} ${./scripts/wsa-routing.nu} ${card}";
        };
      };
    }

    (lib.mkIf cfg.audio.speakerSink {
      services.pipewire.extraConfig.pipewire."50-surface-pro-11-speakers" = {
        "context.objects" = [
          {
            factory = "adapter";
            args = {
              "factory.name" = "api.alsa.pcm.sink";
              "node.name" = "alsa_output.surface-pro-11-speakers";
              "node.description" = "Surface Pro 11 Speakers";
              "media.class" = "Audio/Sink";
              "api.alsa.path" = speakerPcm;
              "api.alsa.disable-mmap" = true;
              "api.alsa.period-size" = 1024;
              "api.alsa.headroom" = 1024;
              "audio.channels" = 4;
              # Slots 0 and 2 are the amplifiers; 1 and 3 are not connected.
              "audio.position" = ["FL" "RL" "FR" "RR"];
              "channelmix.normalize" = false;
              # Mono into both speakers.  Deleting this line gives real stereo,
              # which the relabelling above already allows; it stays because
              # this is the combination that was tested on the machine.
              "channelmix.mix-matrix" = "[ 0.5 0.5, 0.0 0.0, 0.5 0.5, 0.0 0.0 ]";
              "object.linger" = true;
            };
          }
        ];
      };

      services.pipewire.wireplumber.extraConfig."51-surface-pro-11-speakers" = {
        "monitor.alsa.rules" = [
          {
            # Both the UCM profile's sink and the Pro Audio one, either of which
            # would open the same PCM exclusively and lock the sink above out.
            matches = [{"node.name" = "~alsa_output.platform-sound.*";}];
            actions.update-props."node.disabled" = true;
          }
        ];
      };
    })
  ]);
}

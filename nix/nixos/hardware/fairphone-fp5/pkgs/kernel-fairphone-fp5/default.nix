{
  fetchFromGitHub,
  fetchFromGitLab,
  gzip,
  lib,
  linuxKernel,
  stdenv,
  buildPackages,
  ...
}: let
  # Kernel source from `sc7280-mainline` repository.
  kernelSrc = fetchFromGitHub {
    owner = "sc7280-mainline";
    repo = "linux";
    rev = "v6.17.0-sc7280";
    hash = "sha256-k6Fp5Dhy1s7Jnpc1qywHZxmkH2+OAYk1Yy8vSBSyR5k=";
  };

  # Source of postmarketOS `pmaports` repository.
  pmaportsSrc = fetchFromGitLab {
    domain = "gitlab.postmarketos.org";
    owner = "postmarketOS";
    repo = "pmaports";
    rev = "305cddc07f3739747f0662c824e4febccf0e1e28";
    hash = "sha256-QInrf7Sf9j+bB26bsC1hYOnWPz/n5K3WlC50cq7megQ=";
  };

  # Use the kernel configuration from PostmarketOS for the `sc7280` chipset as the base.
  #
  # We override some options that are disabled in PostmarketOS config to make it
  # compatible with NixOS and enable useful functionality:
  # - CONFIG_DMIID: NixOS asserts that this is enabled.
  # - CONFIG_U_SERIAL_CONSOLE: Enables USB serial gadget console output for debugging (module).
  # - CONFIG_USB_G_SERIAL: Classic USB serial gadget driver (module, so it doesn't
  #     lock the DWC3 controller in device mode and block USB host for keyboards).
  # - CONFIG_ANDROID_BINDERFS: Required for Waydroid (Android container support).
  #
  # Additional netfilter/iptables extensions required by NixOS firewall:
  # - CONFIG_NETFILTER_XT_MATCH_PKTTYPE: Packet type matching.
  # - CONFIG_NETFILTER_XT_MATCH_LIMIT: Rate limiting for firewall rules.
  # - CONFIG_NETFILTER_XT_MATCH_RECENT: Recent connections tracking.
  # - CONFIG_NETFILTER_XT_MATCH_STATE: Connection state matching.
  # - CONFIG_NETFILTER_XT_TARGET_LOG: Logging target for firewall rules.
  #
  # Audio subsystem (Fairphone 5 speakers, microphones, SoundWire):
  # - CONFIG_SOUNDWIRE_QCOM: Qualcomm SoundWire controller (bus for WCD938X codec).
  # - CONFIG_SND_SOC_AW88261: AW88261 speaker amplifier codec driver.
  # - CONFIG_SND_SOC_WCD938X_SDW: WCD9385 codec SoundWire interface (mic/headphone/HAC).
  # - CONFIG_SND_SOC_LPASS_RX_MACRO: LPASS RX macro (SoundWire RX path).
  # - CONFIG_SND_SOC_LPASS_TX_MACRO: LPASS TX macro (SoundWire TX path).
  # - CONFIG_SND_SOC_LPASS_VA_MACRO: LPASS VA macro (voice activity / digital mic path).
  #
  # DisplayPort output over USB-C:
  # - CONFIG_TYPEC_DP_ALTMODE: Required for DP Alt Mode over USB-C to work.
  configfile = buildPackages.stdenv.mkDerivation {
    name = "kernel-config";
    src = "${pmaportsSrc}/device/testing/linux-postmarketos-qcom-sc7280/config-postmarketos-qcom-sc7280.aarch64";
    dontUnpack = true;

    buildPhase = ''
      # Read the original config and apply our modifications.
      sed \
        -e 's/# CONFIG_DMIID is not set/CONFIG_DMIID=y/' \
        -e 's/# CONFIG_U_SERIAL_CONSOLE is not set/CONFIG_U_SERIAL_CONSOLE=m/' \
        -e 's/# CONFIG_USB_G_SERIAL is not set/CONFIG_USB_G_SERIAL=m/' \
        -e 's/# CONFIG_ANDROID_BINDERFS is not set/CONFIG_ANDROID_BINDERFS=y/' \
        -e 's/# CONFIG_NETFILTER_XT_MATCH_PKTTYPE is not set/CONFIG_NETFILTER_XT_MATCH_PKTTYPE=m/' \
        -e 's/# CONFIG_NETFILTER_XT_MATCH_LIMIT is not set/CONFIG_NETFILTER_XT_MATCH_LIMIT=m/' \
        -e 's/# CONFIG_NETFILTER_XT_MATCH_RECENT is not set/CONFIG_NETFILTER_XT_MATCH_RECENT=m/' \
        -e 's/# CONFIG_NETFILTER_XT_MATCH_STATE is not set/CONFIG_NETFILTER_XT_MATCH_STATE=m/' \
        -e 's/# CONFIG_NETFILTER_XT_TARGET_LOG is not set/CONFIG_NETFILTER_XT_TARGET_LOG=m/' \
        -e 's/# CONFIG_TYPEC_DP_ALTMODE is not set/CONFIG_TYPEC_DP_ALTMODE=y/' \
        -e 's/# CONFIG_SOUNDWIRE_QCOM is not set/CONFIG_SOUNDWIRE_QCOM=m/' \
        -e 's/# CONFIG_SND_SOC_AW88261 is not set/CONFIG_SND_SOC_AW88261=m/' \
        -e 's/# CONFIG_SND_SOC_WCD938X_SDW is not set/CONFIG_SND_SOC_WCD938X_SDW=m/' \
        -e 's/# CONFIG_SND_SOC_LPASS_RX_MACRO is not set/CONFIG_SND_SOC_LPASS_RX_MACRO=m/' \
        -e 's/# CONFIG_SND_SOC_LPASS_TX_MACRO is not set/CONFIG_SND_SOC_LPASS_TX_MACRO=m/' \
        -e 's/# CONFIG_SND_SOC_LPASS_VA_MACRO is not set/CONFIG_SND_SOC_LPASS_VA_MACRO=m/' \
        $src > config
    '';

    installPhase = ''
      cp config $out
    '';
  };

  # Parse kernel version from Makefile.
  kernelVersion = rec {
    file = "${kernelSrc}/Makefile";
    version = lib.head (lib.match ".*VERSION = ([0-9]+).*" (lib.readFile file));
    patchlevel = lib.head (lib.match ".*PATCHLEVEL = ([0-9]+).*" (lib.readFile file));
    sublevel = lib.head (lib.match ".*SUBLEVEL = ([0-9]+).*" (lib.readFile file));
    string = "${version}.${patchlevel}.${sublevel}";
  };
  modDirVersion = kernelVersion.string;
in
  (linuxKernel.manualConfig {
    inherit lib;

    allowImportFromDerivation = true;
    inherit configfile;
    kernelPatches = [
      {
        # TODO: Remove as soon as `sc7280-mainline` has been updated to v6.18 or later.
        name = "fix-h4-recv-corruption";
        patch = ./patches/fix-h4-recv-corruption.patch;
      }
      {
        name = "hci-qca-drop-unused-event";
        patch = ./patches/hci-qca-drop-unused-event.patch;
      }
    ];
    inherit modDirVersion;
    src = kernelSrc;
    stdenv =
      # Override `stdenv` to produce compressed kernel image target.
      # Use the derivation's own stdenv (which is already a cross stdenv when
      # cross-compiling) so the correct cross-compiler is used.
      stdenv.override {
        hostPlatform =
          stdenv.hostPlatform
          // {
            linux-kernel =
              stdenv.hostPlatform.linux-kernel
              // {
                target = "Image.gz";
                installTarget = "zinstall";
              };
          };
      };
    version = kernelVersion.string;
  }).overrideAttrs (oldAttrs: {
    # Also install the uncompressed `Image` for NixOS compatibility. NixOS expects `Image`
    # to exist, even though we'll use `Image.gz` for boot.
    postInstall =
      (oldAttrs.postInstall or "")
      + ''
        # Decompress Image.gz to Image for NixOS compatibility.
        if [ -f "$out/Image.gz" ] && [ ! -f "$out/Image" ]; then
          echo "Decompressing Image.gz to Image for NixOS compatibility..."
          ${lib.getExe' gzip "gunzip"} -c "$out/Image.gz" > "$out/Image"
        fi
      '';
  })

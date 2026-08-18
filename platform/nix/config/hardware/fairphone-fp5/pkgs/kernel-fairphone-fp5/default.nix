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
  kernelSrc = fetchFromGitHub {
    owner = "sc7280-mainline";
    repo = "linux";
    rev = "v6.17.0-sc7280";
    hash = "sha256-k6Fp5Dhy1s7Jnpc1qywHZxmkH2+OAYk1Yy8vSBSyR5k=";
  };

  pmaportsSrc = fetchFromGitLab {
    domain = "gitlab.postmarketos.org";
    owner = "postmarketOS";
    repo = "pmaports";
    rev = "305cddc07f3739747f0662c824e4febccf0e1e28";
    hash = "sha256-QInrf7Sf9j+bB26bsC1hYOnWPz/n5K3WlC50cq7megQ=";
  };

  # PostmarketOS' sc7280 config as the base, with the options below - all
  # disabled there - turned back on. Why each one:
  #
  # - DMIID: NixOS asserts it is enabled.
  # - U_SERIAL_CONSOLE, USB_G_SERIAL: the USB serial gadget console, for
  #   debugging. As modules, so they do not lock the DWC3 controller in device
  #   mode and block USB host for keyboards.
  # - ANDROID_BINDERFS: Waydroid.
  # - NETFILTER_XT_*: the extensions the NixOS firewall needs.
  # - SOUNDWIRE_QCOM, SND_SOC_*: the speakers, microphones and SoundWire bus.
  # - TYPEC_DP_ALTMODE: DisplayPort over USB-C.
  configfile = buildPackages.stdenv.mkDerivation {
    name = "kernel-config";
    src = "${pmaportsSrc}/device/testing/linux-postmarketos-qcom-sc7280/config-postmarketos-qcom-sc7280.aarch64";
    dontUnpack = true;

    buildPhase = ''
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
      # For the compressed kernel image target. The derivation's own stdenv,
      # already a cross stdenv when cross-compiling, so the compiler is right.
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
    # NixOS expects an uncompressed `Image` to exist, even though `Image.gz` is
    # what boots.
    postInstall =
      (oldAttrs.postInstall or "")
      + ''
        if [ -f "$out/Image.gz" ] && [ ! -f "$out/Image" ]; then
          echo "Decompressing Image.gz to Image for NixOS compatibility..."
          ${lib.getExe' gzip "gunzip"} -c "$out/Image.gz" > "$out/Image"
        fi
      '';
  })

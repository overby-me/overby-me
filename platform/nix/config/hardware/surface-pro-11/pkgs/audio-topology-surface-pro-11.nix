# The AudioReach topology the Surface Pro 11 sound card asks for.
#
# audioreach_tplg_init composes qcom/<driver_name>/<card_name>-tplg.bin from the
# machine driver's match data and the devicetree `model`, and refuses to probe
# without the result: no topology, no card, no ALSA device at all.  linux-firmware
# carries a topology per X1E80100 machine but none under this machine's name.
#
# A rename, not a build.  The topology ooaklee/linux-surface-pro-11-oe publishes
# for this machine, and its sp11-audio-topology.sh builds from
# linux-msm/audioreach-topology's X1E80100-CRD.m4 through m4 and alsatplg, is
# byte-identical to the X1E80100-CRD-tplg.bin already here: sha256
# e9c74273a3b01bfed3ae53ed80694c35c9b24faed367e31b595b9fb1b95eadee for both,
# against linux-firmware 20260622.  So no m4 toolchain and no clone in the
# closure.
#
# Being the shared CRD topology, it carries more than this devicetree connects:
# RX_CODEC_DMA_RX_0 (WCD939x headphone), TX_CODEC_DMA_TX_3 (headset mic) and
# DISPLAY_PORT_RX_0-7 are all in it and all unreachable until the devicetree
# declares the matching DAI links.
#
# Delete this if a Denali topology is ever published to linux-firmware under its
# own name; request_firmware will find that without help.
{
  linux-firmware,
  runCommand,
}:
runCommand "audio-topology-surface-pro-11-${linux-firmware.version}" {
  inherit (linux-firmware) version;

  meta = {
    description = "AudioReach DSP topology for the Microsoft Surface Pro 11";
    longDescription = ''
      The X1E80100 CRD AudioReach topology from linux-firmware, installed under
      the file name the Surface Pro 11's sound card requests. The two are
      byte-identical; only the name differs.
    '';
    inherit (linux-firmware.meta) license;
    maintainers = [];
    platforms = ["aarch64-linux"];
  };
} ''
  install -Dm444 \
    ${linux-firmware}/lib/firmware/qcom/x1e80100/X1E80100-CRD-tplg.bin \
    $out/lib/firmware/qcom/x1e80100/X1E80100-Microsoft-Surface-Pro-11-tplg.bin
''

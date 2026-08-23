# alsa-ucm-conf with a Surface Pro 11 profile added.
#
# Copied rather than symlinkJoined, because registering the board means
# appending to alsa-ucm-conf's own x1e80100 DMI matcher, and a symlinkJoin
# cannot edit a file it links.
#
# Consumers reach this through ALSA_CONFIG_UCM2 rather than through alsa-lib's
# built-in directory, so nothing is rebuilt against it.  See audio.nix.
{
  alsa-ucm-conf,
  runCommand,
}:
runCommand "alsa-ucm-conf-surface-pro-11-${alsa-ucm-conf.version}" {
  inherit (alsa-ucm-conf) version;

  meta =
    alsa-ucm-conf.meta
    // {
      description = "ALSA Use Case Manager configuration, with a Microsoft Surface Pro 11 profile";
      maintainers = [];
    };
} ''
  mkdir -p $out
  cp -r ${alsa-ucm-conf}/share $out/share
  chmod -R u+w $out/share

  board=$out/share/alsa/ucm2/Qualcomm/x1e80100

  install -m 444 ${../ucm/MICROSOFT-Surface-Pro-11.conf} $board/MICROSOFT-Surface-Pro-11.conf
  install -m 444 ${../ucm/Surface11-HiFi.conf} $board/Surface11-HiFi.conf

  if ! grep -q 'Define.DMI_info' $board/x1e80100.conf; then
    echo "alsa-ucm-conf's x1e80100 matcher no longer defines DMI_info." >&2
    echo "ucm/x1e80100-surface-pro-11.conf appends a block that reads it." >&2
    exit 1
  fi
  cat ${../ucm/x1e80100-surface-pro-11.conf} >> $board/x1e80100.conf
''

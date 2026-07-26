{
  name = "26-SYSTEMCTL";
  # The interactive `systemctl edit` hunks stay masked: they drive EDITOR
  # through util-linux `script` for TTY allocation, and script(1) hangs under
  # rust-systemd as PID 1 (a known separate bug in its parent-side termios/poll
  # setup). The `override.conf` cmp assertions go with them, since they check
  # the result of those edits.
  #
  # The `systemctl edit --global` mask is REMOVED as of 2026-07-27: --global is
  # implemented (systemctl/src/main.rs handles it in the arg parser, in `cat
  # --global`, and in the drop-in base-directory selection).
  patchScript = ''
    # Interactive EDITOR tests need `script` for a TTY; script(1) hangs under PID 1.
    sed -i "/EDITOR=.*script -ec/d" TEST-26-SYSTEMCTL.sh
    sed -i '/^\[ ! -e.*override\.conf/d' TEST-26-SYSTEMCTL.sh
    sed -i '/^printf.*>"+4"$/d' TEST-26-SYSTEMCTL.sh
    sed -i '/^printf.*cmp.*\.d\/override\.conf"$/d' TEST-26-SYSTEMCTL.sh
  '';
}

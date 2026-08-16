{
  name = "26-SYSTEMCTL";
  # The interactive `systemctl edit` hunks run under oxidized-systemd. Two fixes made
  # them work: the drop-in scaffold no longer seeds a `[Service]` header (a no-op
  # `EDITOR=true` edit discards, so override.conf stays absent), and the editor is
  # invoked as `$EDITOR +4 <path>` like upstream `run_editor`, so the `EDITOR=mv`
  # test hack (`mv +4 <path>`) swaps in its prepared file. The `user@0` edit (a
  # no-op edit of a template instance, upstream's #26483 double-free regression)
  # runs too: Rust cannot double-free, and a drop-in edit needs no running user
  # manager.
  #
  # The only adaptation left is dropping the util-linux `script` TTY wrapper:
  # oxidized-systemd's `systemctl edit` needs no controlling TTY here, and script(1)
  # hangs under oxidized-systemd as PID 1 (a separate parent-side termios/poll bug).
  #
  # The `systemctl edit --global` mask was removed 2026-07-27; --global works.
  patchScript = ''
    sed -i "s|script -ec '\(systemctl edit [^']*\)' /dev/null|\1|g" TEST-26-SYSTEMCTL.sh
  '';
}

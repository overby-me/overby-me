{
  name = "26-SYSTEMCTL";
  # The interactive `systemctl edit` hunks now run under rust-systemd. Two fixes
  # made them work: the drop-in scaffold no longer seeds a `[Service]` header (a
  # no-op `EDITOR=true` edit discards, so override.conf stays absent), and the
  # editor is invoked as `$EDITOR +4 <path>` like upstream `run_editor`, so the
  # `EDITOR=mv` test hack (`mv +4 <path>`) swaps in its prepared file.
  #
  # We still drop the util-linux `script` TTY wrapper from these lines:
  # rust-systemd's `systemctl edit` needs no controlling TTY here, and script(1)
  # hangs under rust-systemd as PID 1 (a separate parent-side termios/poll bug).
  # Only the `user@0` edit stays masked, since it needs the user manager.
  #
  # The `systemctl edit --global` mask was removed 2026-07-27; --global works.
  patchScript = ''
    sed -i "s|script -ec '\(systemctl edit [^']*\)' /dev/null|\1|g" TEST-26-SYSTEMCTL.sh
    sed -i "/systemctl edit user@0/d" TEST-26-SYSTEMCTL.sh
  '';
}

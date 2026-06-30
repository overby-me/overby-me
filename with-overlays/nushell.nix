# Patch crossterm cursor position lag fix (crossterm-rs/crossterm#1024)
# into nushell's vendored crossterm dependencies.
# Fixes: https://github.com/nushell/nushell/issues/17179
_: prev: {
  pkgsUnstable =
    prev.pkgsUnstable
    // {
      nushell = prev.pkgsUnstable.nushell.overrideAttrs (old: {
        preConfigure =
          (old.preConfigure or "")
          + ''
            # Patch crossterm read_position_raw to drain stale cursor position events
            for dir in $cargoDepsCopy/crossterm-*/; do
              f="$dir/src/cursor/sys/unix.rs"
              if [ -f "$f" ]; then
                substituteInPlace "$f" \
                  --replace-fail \
                    'fn read_position_raw() -> io::Result<(u16, u16)> {' \
                    'fn read_position_raw() -> io::Result<(u16, u16)> {
                // Discard any buffered cursor-position replies from earlier ESC[6n requests so the
                // position returned below corresponds to the fresh request we are about to send.
                while let Ok(true) = poll_internal(Some(Duration::ZERO), &CursorPositionFilter) {
                    let _ = read_internal(&CursorPositionFilter);
                }
            '
              fi
            done
          '';
      });
    };
}

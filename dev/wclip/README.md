# wclip

`wclip` is an [`xclip`](https://github.com/astrand/xclip)-style clipboard
tool for **Wayland**, written in pure Rust. It reads data into a selection or
prints the current selection to standard output, just like `xclip` does on X11.

It speaks the Wayland wire protocol **directly over the compositor socket** —
there is no dependency on `libwayland` or any C library (only `libc` for the
`sendmsg`/`recvmsg` file-descriptor passing that the protocol requires). The
design is inspired by [`wl-clipboard-rs`](https://github.com/YaLTeR/wl-clipboard-rs).

## How it works

A command-line tool has no window and therefore can never hold keyboard focus,
so the core Wayland clipboard protocols (`wl_data_device`, primary-selection)
are unusable — they only deliver the selection to the focused surface. Instead
`wclip` uses one of the dedicated, focus-less *data-control* protocols:

| Protocol | Used by | Notes |
|-|-|-|
| `ext-data-control-v1` | GNOME/Mutter, the standardized successor | preferred |
| `wlr-data-control-unstable-v1` | wlroots (sway, Hyprland, …) | fallback |

Both are detected at runtime and, since they share an identical opcode layout,
drive the same code path. `ext-data-control` is preferred when both are present.

When **copying**, `wclip` installs a data source and then forks a small
background process that keeps serving the data to pasting clients (exactly like
`xclip` and `wl-copy`), so your shell prompt returns immediately. Use
`-foreground` to keep it in the foreground instead.

## Building

```sh
nix build .#rust-wclip
./result/bin/wclip -help
```

Or with Cargo:

```sh
cargo build --release
```

## Usage

```text
wclip [OPTION]... [FILE]...

  -i, -in            read data into a selection (default)
  -o, -out           write the selection to standard output
  -sel, -selection SEL   'primary' (default) or 'clipboard' (abbreviations ok)
  -t, -target TARGET     MIME type; with -o, 'TARGETS' lists available types
  -l, -loops N           serve N paste requests then exit (0 = unlimited)
  -r, -rmlastnl          strip one trailing newline from the data
  -f, -filter            when copying, also echo the input to stdout
  -foreground            keep serving in the foreground (do not fork)
  -d, -display NAME      Wayland display (default $WAYLAND_DISPLAY)
  -h, -help / -version
```

### Examples

```sh
# Copy a file to the clipboard
wclip -selection clipboard < file.txt

# Copy some text to the clipboard
echo "hello" | wclip -sel c

# Paste the clipboard to stdout
wclip -o -sel c

# List the MIME types the clipboard currently offers
wclip -o -sel c -t TARGETS

# Copy a PNG with an explicit type
wclip -sel c -t image/png < image.png
```

> **Note** — like `xclip`, the **default selection is PRIMARY** (the
> middle-click buffer), *not* the clipboard. Most applications paste with
> Ctrl+V from the clipboard, so you usually want `-selection clipboard`
> (`-sel c` for short).

## Compatibility

`wclip` requires the compositor to implement `ext-data-control-v1` or
`wlr-data-control-unstable-v1`. wlroots compositors (sway, Hyprland, river, …)
support the latter; recent GNOME/KDE support the former. On a compositor that
exposes neither, `wclip` exits with a clear error — the core protocols cannot
serve a focus-less client, which is the same limitation `wl-clipboard` has.

## Running the test suite

The protocol logic is covered by roundtrip tests that drive the real client
against an in-process mock compositor over a `socketpair` (no display needed);
they run automatically during `nix build .#rust-wclip` and via `cargo test`.

Additional command-line behaviour is checked in the Nix sandbox:

```sh
# Run a single test
nix build .#checks.x86_64-linux.rust-wclip-test-{name}

# View failure output
nix log .#checks.x86_64-linux.rust-wclip-test-{name}
```

See `default.nix` for the full list of test names.

## License

MIT

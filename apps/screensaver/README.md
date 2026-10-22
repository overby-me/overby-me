# Screensaver

[screensaver.overby.me](https://screensaver.overby.me): Rust ports of the
[XScreenSaver](https://www.jwz.org/xscreensaver/) hacks, running in the browser.
It used to live at `overby.me/screensaver`, which now redirects here.

## Routes

- `/`: Picks one of the ports at random and redirects to it
- `/<name>`: Runs that screensaver, with its options behind a button
  (see [`xscreensaver/`](./xscreensaver/README.md))
- `/<name>?images=@handle`: Savers that work on a picture take it from that
  account's posts; `?images=%23tag` takes them live off the firehose
- `/<name>?text=@handle`: Savers that show text take it the same way

## Getting Started

The Nix dev shell provides the toolchain (`dx`, a Rust toolchain with the
`wasm32-unknown-unknown` target, `wasm-bindgen-cli`, and `binaryen`):

```sh
nix develop .#screensaver
just dev
```

### Build

```sh
just build
```

The static site is written to `target/dx/screensaver/release/web/public`. The
release build gives each screensaver its own wasm chunk, so opening one
downloads only that one. See [`xscreensaver/README.md`](./xscreensaver/README.md)
for how that is wired and what it measures.

### Test

```sh
just test
```

Runs the ports headlessly: every saver renders frames at several window sizes
and option values with no browser involved. `just browser` renders the
Shadertoy savers in headless Chromium after a build.

### Deploy

```sh
STATICHOST_APIKEY=... just deploy
```

Uploads the build to the `screensaver-overby-me` site on
[statichost.eu](https://www.statichost.eu).

## License

Licensed under AGPL-3.0.

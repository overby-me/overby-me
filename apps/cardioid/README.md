# Cardioid

[cardioid.overby.me](https://cardioid.overby.me): an epicyclic curve tracer.
A sum of rotating arms draws the curve, and a times-table mode draws modular
chords on a circle. It used to live at `overby.me/cardioid`, which now
redirects here.

The panel starts as a two-arm cardioid, with the advanced knobs in collapsed
sections. Every setting is kept in the query string, so a configured curve is
a shareable link.

## Getting Started

The Nix dev shell provides the toolchain (`dx`, a Rust toolchain with the
`wasm32-unknown-unknown` target, `wasm-bindgen-cli`, and `binaryen`):

```sh
nix develop .#cardioid
just dev
```

### Build

```sh
just build
```

The static site is written to `target/dx/cardioid/release/web/public`.

### Deploy

```sh
STATICHOST_APIKEY=... just deploy
```

Uploads the build to the `cardioid` site on
[statichost.eu](https://www.statichost.eu).

## License

Licensed under AGPL-3.0.

# Homepage

Personal homepage for [overby.me](https://overby.me) — an interactive 3D graph visualization that maps out online presence, interests, and connections.

## Overview

The landing page renders a 3D force-directed graph where nodes represent profiles, platforms, and life categories (Commerce, Improve, Connect, Immerse, Give). Clicking a node navigates to the corresponding URL; nodes can be dragged, and the camera orbits and zooms.

The graph is a Rust/WebAssembly rewrite of the original React version: it draws directly to WebGL and runs a faithful port of [d3-force-3d](https://github.com/vasturiano/d3-force-3d) (the force model [react-force-graph-3d](https://github.com/vasturiano/react-force-graph) uses) for the layout.

Additional routes:

- `/screensaver` — Picks one of the [XScreenSaver](https://www.jwz.org/xscreensaver/)
  ports at random and redirects to it
- `/screensaver/<name>` — Runs that screensaver, with its options behind a button
  (see [`xscreensaver/`](./xscreensaver/README.md))
- `/screensaver/<name>?images=@handle` — Savers that work on a picture take it
  from that account's posts; `?images=%23tag` takes them live off the firehose
- `/cardioid` — An epicyclic curve tracer
- `/search` — Redirects search queries to [Startpage](https://startpage.com)
- `/x` — Redirects X/Twitter links through [xcancel.com](https://xcancel.com)
- `/yt` — Embeds YouTube videos in a clean full-screen player

## Tech Stack

- **[Dioxus](https://dioxuslabs.com/)** (Rust) compiled to **WebAssembly**
- **WebGL** via `web-sys` for the custom 3D renderer, with **[glam](https://github.com/bitshifter/glam-rs)** for math
- Self-hosted **[Space Grotesk](https://fonts.google.com/specimen/Space+Grotesk)** for the graph labels
- **Dioxus Router** for client-side routing
- **[Nix](https://nixos.org/)** for reproducible dev environments
- **[Just](https://github.com/casey/just)** as a command runner, driving the **[Dioxus CLI](https://crates.io/crates/dioxus-cli)** (`dx`)

## Getting Started

### Prerequisites

The Nix dev shell provides the full toolchain (`dx`, a Rust toolchain with the
`wasm32-unknown-unknown` target, `wasm-bindgen-cli`, and `binaryen`):

```sh
nix develop .#homepage
```

### Development

```sh
just dev
```

### Build

```sh
just build
```

The static site is written to `target/dx/homepage/release/web/public`.

The release build gives each screensaver its own wasm chunk, so opening one
downloads only that one. See [`xscreensaver/README.md`](./xscreensaver/README.md)
for how that is wired and what it measures.

### Test

```sh
just test
```

Runs the screensaver ports headlessly: every saver renders frames at several
window sizes and option values with no browser involved.

### Clean

```sh
just clean
```

## License

Licensed under AGPL-3.0.

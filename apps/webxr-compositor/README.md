# webxr-compositor

Run Wayland applications in a web browser. A native Rust host owns a real
Wayland socket; connected apps are streamed as pixels to a Dioxus/WASM
frontend that composites them, first as ordinary draggable windows (normal
mode), eventually as panels in a WebXR scene.

Inspired by [greenfield](https://github.com/udevbe/greenfield), a browser
compositor that runs arbitrary Wayland apps, and
[vr-on-rails](https://github.com/prozum/vr-on-rails), a VR terminal
prototype.

## Architecture

| Piece | Crate | Runs | Role |
|-|-|-|-|
| Host | `host/` (`webxr-compositor-host`) | native | Owns `$XDG_RUNTIME_DIR/wayland-*` via smithay, reads client buffers, serves HTTP and the WebSocket |
| Protocol | `protocol/` | both | postcard-encoded messages: RGBA damage frames out, input events in |
| Frontend | `src/` (`webxr-compositor`) | browser | Dioxus app: one canvas per window, pointer and keyboard capture, later the WebXR scene |

The host and the frontend are separate cargo workspaces so smithay and the
server stack never enter the wasm dependency graph.

## Status

The host serves the built frontend over HTTP and completes the wire-protocol
hello with the page over /ws (checked end to end by `just browser`). No
Wayland socket yet. Roadmap: Wayland globals, shm surface pipeline, input,
window management, damage optimization, clipboard and cursors, GTK-class
apps, GPU clients (Zed), WebXR mode.

## Run

```sh
just run        # dx-build the frontend, then serve it from the host
just dev        # frontend-only hot reload (dx serve)
just test       # both workspaces
just lint
just browser    # headless-chromium check of the page against the real host
```

The host reads `WEBXR_COMPOSITOR_LISTEN` (default `127.0.0.1:8370`) and
`WEBXR_COMPOSITOR_WEB_ROOT` (default: the dx release output path).

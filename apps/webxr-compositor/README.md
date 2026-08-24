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

Wayland apps draw in the browser. The host runs a smithay compositor on its
own thread with a real socket (`WEBXR_COMPOSITOR_WAYLAND_DISPLAY`, else
auto-named) advertising wl_compositor, wl_subcompositor, wl_shm, wl_seat,
wl_output + xdg-output, xdg_wm_base and wl_data_device_manager
(`just wayland`). Committed shm buffers are converted to RGBA, broadcast to
every connected page and painted onto a per-window canvas, with frame
callbacks acked at 60 Hz and late-joining browsers resynced (`just surface`
proves pixel-exact colours and animation with the bundled `checker` client;
`just browser` proves the hello). Pointer and keyboard input flow back:
clicking a window focuses it, canvas events become seat pointer events, and
a W3C-code-to-evdev keymap feeds the xkb keyboard, so typing into a real
terminal works (`just input` clicks and types into foot and asserts the
echo). Windows behave like windows: titlebar drag, click to focus and raise
(with xdg activated state), a close button, a resize handle that becomes an
xdg configure clamped to the client's min/max, server-side decorations
forced via zxdg-decoration, and reload resync (`just windows` drives all of
it with checker and foot side by side). Roadmap: damage optimization,
clipboard and cursors, GTK-class apps, GPU clients (Zed), WebXR mode.

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

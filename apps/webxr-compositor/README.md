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
it with checker and foot side by side). Frames carry client damage rather
than full surfaces: the host crops commits to the damage bounding box
(falling back to full frames on resize, scale != 1 or near-full damage),
patches its stored frame for resync, and withholds frame callbacks while
any browser has over 32 MB queued (`just damage` measures typing into foot
at about 600 bytes per frame against a 1.3 MB full surface). Occlusion
culling is not done: the host does not know the browser's window layout.

Clipboard and cursors work: selection offers follow keyboard focus, a
client copy is read through a pipe, mirrored to the page (and to
`navigator.clipboard` where the browser permits) and taken over by the
host so it outlives the client; pastes are served from the host copy, and
the browser clipboard is pushed to the host on each focus click (needs the
clipboard-read permission; headless chromium denies it, so `just clipboard`
proves the client-to-host-to-page-to-other-client circuit instead).
cursor-shape-v1 maps named cursors straight to CSS cursors; surface-drawn
cursors fall back to the arrow. Primary selection passes between clients
through smithay.

GTK4 apps run: xdg popups (menus, popovers, tooltips) render as overlays
anchored in their parent surface, nested popups nest, GTK's
unmap-not-destroy popdown is handled, and a press outside the popup chain
dismisses it (`just gtk` drives gnome-calculator's hamburger menu open and
closed). Not there yet: wp_viewporter and fractional-scale are deliberately
not advertised (clients render correctly at scale 1 instead of being lied
to), subsurfaces are not composited (GTK4 does not need them for normal
windows), popups never receive keyboard focus (menu arrow-key navigation
stays with the mouse), and xdg_activation is absent. Roadmap: GPU clients
(Zed), WebXR mode.

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

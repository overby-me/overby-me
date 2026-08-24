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
stays with the mouse), and xdg_activation is absent.

Zed runs. With Mesa's software Vulkan (lavapipe presents through wl_shm),
Zed opens, renders its full UI and accepts mouse and keyboard input in the
browser (`just zed` boots an isolated stateless Zed, accepts the trust
dialog and types into the buffer; it wants `zeditor` on PATH and resolves
the lavapipe ICD from nixpkgs#mesa). Hardware GPU clients are not
supported: zwp_linux_dmabuf is not advertised, so Vulkan and GL clients
fall back to software rendering, which is the honest limit of a
pixel-streaming compositor until host-side readback exists.

The 3D mode is live: the hidden flat desk keeps painting the per-window
canvases, and a raw-WebGL scene draws them as textures on quads along an
arc (1000 px to the metre, popups floating in front of their parent).
The "3D view" button flips into a mouse-look preview where clicking a
quad routes real pointer input to that window; "enter VR" (shown when
navigator.xr exists) starts an immersive-vr session over the same scene,
with per-eye view and projection taken from the XR pose through dynamic
JS calls. `just xr` proves the pipeline by sampling the checker palette
off the WebGL canvas and toggling back to the flat desk, and `just zed`
now also types into Zed through the 3D view's ray picking. In the
immersive session, rendering targets the XRWebGLLayer framebuffer, the
preview loop yields the context while the session runs and takes it back
on end, and the first controller steers the pointer: its target ray is
picked against the quads every frame and select press/release become
pointer buttons on the hit window. The session path follows the WebXR
spec but remains unverified: this chromium is built without the XR
device service (`navigator.xr` is absent even with the blink features
forced), so proving it needs a WebXR-enabled browser and a headset.

## Run

```sh
just run        # dx-build the frontend, then serve it from the host
just dev        # frontend-only hot reload (dx serve)
nix run .#webxr-compositor-app   # hermetic build, bundle served from the store
```

Then connect any Wayland app to the socket the host logs:

```sh
WAYLAND_DISPLAY=wayland-<n> foot
```

## Checks

```sh
just test       # unit tests, both workspaces
just lint       # clippy, both workspaces
just browser    # the page completes the hello against the real host
just wayland    # wayland-info sees every advertised global
just surface    # checker pixels reach the canvas and animate
just input      # typing echoes in foot
just windows    # drag, focus, raise, resize, close, reload-resync
just damage     # typed frames stay hundreds of bytes, not megabytes
just clipboard  # copy in one foot, paste into another via the host
just gtk        # gnome-calculator's popover menu opens and dismisses
just zed        # Zed (software Vulkan) renders and accepts typing
just xr         # the 3D scene shows live window content
```

`nix build .#webxr-compositor-frontend` and `.#webxr-compositor-app` build
both halves hermetically (the flake exposes this app's packages under the
`webxr-compositor-` prefix).

The host reads `WEBXR_COMPOSITOR_LISTEN` (default `127.0.0.1:8370`) and
`WEBXR_COMPOSITOR_WEB_ROOT` (default: the dx release output path).

Frames travel lz4-compressed whenever that shrinks them (solid UI content
collapses by two orders of magnitude), so the link is usable beyond
loopback. Security follows exposure: on loopback the default stays plain
HTTP, but any other bind address switches to TLS (self-signed unless
`WEBXR_COMPOSITOR_CERT`/`_KEY` name real ones) plus an access token
(`WEBXR_COMPOSITOR_TOKEN`, else generated) that the printed URL carries
and every WebSocket connect must present; `WEBXR_COMPOSITOR_TLS=1` forces
secure mode on loopback too, and `WEBXR_COMPOSITOR_INSECURE=1` is the
explicit opt-out (`just tls` proves wss works and wrong tokens bounce).

Sustained motion streams as video: fifteen full-surface commits inside a
second flip that surface to H.264 (openh264 host-side, WebCodecs
VideoDecoder in the page painting onto the same canvas), and thirty quiet
commits, a resize or an encode error flip it back to damage rects from a
clean full repaint. Pages advertise decode support in their hello, one
holdout keeps everything on rects, and joiners force the next frame to a
keyframe. `just video` measures the fast checker at around 600x under raw
(52 KB wire for 32 MB of frames) with the decoded quadrants still
palette-correct within codec tolerance.

Hardware GPU clients work: with a usable render node the host advertises
zwp_linux_dmabuf v4 with default feedback (without the main device in the
feedback, mesa silently falls back to software), imports committed
dmabufs through EGL/GLES, blits them into an offscreen target (external
textures cannot back an FBO directly) and reads RGBA back into the same
pipeline, caching one imported texture per client buffer since importing
per frame exhausts GL within seconds. `just gpu` proves es2gears on the
real GPU end to end, and Zed on the real Vulkan driver commits hardware
dmabufs the same way. Hosts without a render node or the EGL runtime
come up unchanged with dmabuf off; the nix wrapper carries libglvnd for
the runtime.

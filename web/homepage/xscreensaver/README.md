# xscreensaver

Rust ports of the [XScreenSaver](https://www.jwz.org/xscreensaver/) hacks,
served at `overby.me/screensaver/<name>`.

Upstream ships 314 screensavers. They fall into three groups, each needing a
different runtime:

| Tier | Savers | Upstream | Runtime it needs | State |
|-|-|-|-|-|
| 2D | 141 | `hacks/*.c`, Xlib | software framebuffer + Xlib façade | in progress |
| Shadertoy | 30 | `hacks/glx/glsl/*.glsl` | WebGL2 multi-pass runner | not started |
| OpenGL | 138 | `hacks/glx/*.c`, GL 1.x | immediate-mode emulation over WebGL2 | not started |

`webcollage` and `vidwhacker` are not portable: they scrape images off the live
web. `co____9`, `companioncube` and `mismunch` are aliases or variants of other
savers.

## How a port works

A 2D hack upstream is five C functions and a table:

```c
static void *NAME_init    (Display *, Window);
static unsigned long NAME_draw (Display *, Window, void *closure);
static void NAME_reshape  (Display *, Window, void *, unsigned, unsigned);
static Bool NAME_event    (Display *, Window, void *, XEvent *);
static void NAME_free     (Display *, Window, void *);
XSCREENSAVER_MODULE ("Name", name)
```

`init` returns the hack's state, `draw` renders one step and returns how many
microseconds it wants to wait, and the driver loops. The port keeps that shape:
the state is a struct, `Screenhack` is the trait, `free` is `Drop`, and the
module table is a `SaverDef`.

The Xlib side is implemented in software. `runtime::Fb` is a `Vec<u32>` that
`fill_rectangle`, `draw_line`, `fill_polygon` and friends rasterise into, and
the host blits it to a canvas once per frame. That is not a performance
compromise, it is the cheaper option: the hacks read pixels back
(`XGetImage`), draw in XOR (`XSetFunction`) and copy between pixmaps
constantly, all of which are awkward and slow through a canvas context and
trivial over a pixel buffer. It also means the whole collection is testable
without a browser.

### Adding a saver

1. Copy `hacks/<name>.c` to `src/hacks2d/<name>.rs`, keeping the upstream
   copyright header verbatim.
2. `struct state` becomes a Rust struct; `NAME_init` becomes the `new` function
   the `SaverDef` points at; `NAME_draw/reshape/event` become the `Screenhack`
   impl.
3. Copy `NAME_defaults[]` across verbatim. It is parsed at runtime, so the C
   strings work as-is.
4. Translate the knobs from `hacks/config/<name>.xml` into the `opts` table.
   `arg-set` on a `<boolean>` means it defaults off, `arg-unset` means on;
   `convert="invert"` on a slider means `.inverted()`.
5. Replace `get_integer_resource (dpy, "delay", "Integer")` with
   `d.res.int("delay")`, and so on.
6. Register it in `src/hacks2d/mod.rs` and in `../src/pages/savers.rs`.
7. Look at it: `just shot <name>` renders frames to a PPM you can compare
   against the original (or against the video linked in its `SaverDef::about`).

The tests in `src/hacks2d/mod.rs` then cover the new saver automatically: they
check every registered saver draws something, keeps changing, is reproducible
from its seed, and survives degenerate window sizes, mid-run resizes, pointer
events and both extremes of every option it declares.

## Code splitting

Each saver is its own lazily-loaded wasm chunk, declared in
`../src/pages/savers.rs`. Two things about that are worth knowing before
changing it:

- **`#[component(lazy)]` cannot be used.** It hardcodes the split module name to
  `"lazy"`, so every lazy component in the app shares one chunk and opening any
  saver would download all of them. The `lazy_loader!` macro underneath it takes
  a module name, so each saver declares its own.
- **The chunk returns data, not UI.** `wasm-split` 0.7.9 never emits a shared
  chunk (`build_split_chunks` computes an empty set), so anything reachable from
  two split modules but not from `main` is copied into *both*. Handing back a
  bare `SaverDef` keeps the shared runtime in the main module and each chunk
  down to the one hack in it. Nothing in `savers.rs` may call
  `xscreensaver::all()` or `find()`, which reference every saver.

The `split` cargo feature deliberately depends on `wasm-splitter` directly
rather than on `dioxus/wasm-split`: enabling the dioxus feature also makes the
router split every route, which is both more than we want and enough to crash
dioxus-cli 0.7.9's splitter (`Failed to find data symbol`, reproducible with no
screensaver code involved).

### It does not pay for itself yet

`just build-split` works: the chunks are emitted, and loading
`/screensaver/munch` fetches `homepage_bg.wasm` and `module_1_munch_body.wasm`
and nothing else. But measured on a clean build with three savers registered:

| Build | main.wasm | chunks |
|-|-|-|
| no savers at all | 935,996 (376,517 gz) | n/a |
| savers, no split | 1,027,475 (413,569 gz) | n/a |
| savers, split | 1,041,033 (418,732 gz) | 3 x ~485 bytes |

The savers plus the shared runtime cost 91,479 bytes (37,052 gzipped) in the
main module, and splitting moves ~1.5 KB of that out while adding ~13.5 KB of
splitter overhead. So `just build` does not split today.

The reason is the split boundary. Each chunk's entry point returns a
`&'static SaverDef`, and everything that *runs* a hack (`Runner`, and the
indirect call through `SaverDef::new`) lives in the main module, so the
splitter attributes the hack's code to main and leaves a pointer in the chunk.
Fixing that means moving the boundary from returning data to running code,
without going back to duplicating the whole runtime into every chunk. It is
worth solving before the bulk of the 141 ports land, because otherwise the main
module grows by every saver ever added.

`--debug-symbols false` applies either way: DWARF is ~90 KB gzipped and needs a
browser extension to read.

## Licence

The hacks carry a permissive MIT-style notice ("Permission to use, copy, modify,
distribute, and sell this software..."), which is why every ported file keeps
its original copyright header. The panel credits each saver's author and links
its upstream demo video.

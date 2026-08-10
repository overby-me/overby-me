# xscreensaver

Rust ports of the [XScreenSaver](https://www.jwz.org/xscreensaver/) hacks,
served at `overby.me/screensaver/<name>`.

Upstream ships 314 screensavers. They fall into three groups, each needing a
different runtime:

| Tier | Savers | Upstream | Runtime it needs | State |
|-|-|-|-|-|
| 2D | 142 | `hacks/*.c`, Xlib | software framebuffer + Xlib façade | in progress (139) |
| Shadertoy | 30 | `hacks/glx/glsl/*.glsl` | WebGL2 multi-pass runner | not started |
| OpenGL | 136 | `hacks/glx/*.c`, GL 1.x | immediate-mode emulation over WebGL2 | not started |

`webcollage` and `vidwhacker` are not portable: they scrape images off the live
web. `co____9`, `companioncube` and `mismunch` are aliases or variants of other
savers.

Every 2D hack that needs nothing but the runtime is ported, and so is every one
that needed a runtime piece worth building. The three left:

| Blocked on | Savers |
|-|-|
| the fifteen pictures and the dozen machines it imitates | `bsod` |
| a JPEG decoder | `glitchpeg` |
| a 6502 emulator | `m6502` |

`testx11` also has a config file and a `hacks/*.c`, but it is upstream's test
harness for the Xlib layer rather than a screen saver, so it is not counted
above.

## Television

`runtime::analogtv` is upstream's `analogtv.c`, and it is not a filter. A hack
that uses it draws its picture, and the module *modulates that into a composite
video signal and demodulates it again*: 912 samples a line at four times the
colour subcarrier, with sync, burst, picture and porches where the standard
puts them. Everything that makes it look like television falls out of doing
that honestly, including the colour fringes, the softness, the bloom, and a
picture that bends when the signal is weak.

Upstream splits the work over a thread pool and carries a whole path for
eight-bit colormapped displays. Neither applies here, so this is one loop over
TrueColor.

## Text

`runtime::font` is one bitmap font compiled in, the `gallant12x22` that
XScreenSaver already bundles for its own console-ish hacks, so it arrives with
the same provenance as everything else here. A hack that asks for a font gets
a whole magnification of it: nearest to the requested size, never below one.

That is a real divergence and there is no way around it short of writing a
glyph rasteriser. It is a smaller one than it sounds, because these hacks lay
themselves out *from* the metrics they are handed rather than assuming a size.
Ask for twenty-four point and get twenty-two pixels and the page simply has the
columns that implies.

The words themselves come from `runtime::text`, which is the same channel
`runtime::image` is: a hack reads characters, the host pushes text in when it
has some, and with no host the compiled-in passage is served instead. Upstream
reads bytes from a pipe to `xscreensaver-text` and copes with there being none
ready yet, so this yields nothing rather than blocking, the same way.

A hack also tells the source how wide its page is (`textclient_reshape`), and
upstream's folds its output to that. So does the fallback: several hacks lay the
words out exactly as they arrive and would otherwise run a paragraph off the
side of the screen.

Upstream's pipe is a pty, so its line endings are a terminal's: a carriage
return in front of every line feed. The channel puts that in, because the hacks
are written for it. Two of them feed these bytes to a terminal emulator, where a
line feed moves down a line and only a carriage return goes back to the left
margin, so without it the text walks off the right-hand side one line at a time;
the others already have code that expects to see the pair.

## Terminals

`runtime::tty` is upstream's `ansi-tty.c`: a VT100 that renders to a character
grid rather than to a screen. Bytes go in, and a hack reads the grid off and
draws it however it draws things, which is how one emulator serves both a
phosphor tube and an Apple II. It knows the sequences a real one does,
including the scrolling region, the line-drawing character set, and the Last
Column Flag, which is the rule that a character printed in the last column
leaves the cursor sitting on top of it and only the character *after* that
wraps.

Upstream carries a great deal of logging behind a debug level, naming every
sequence it recognises and every one it does not. There is nowhere to log to
here, so that is left out; the commands themselves are all present, including
the ones whose implementation upstream is to do nothing.

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
module table is a `SaverDef` plus a `start` entry point.

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
2. `struct state` becomes a Rust struct; `NAME_init` becomes `init`;
   `NAME_draw/reshape/event` become the `Screenhack` impl. Add the module's
   `start`, which is `Runner::start(&DEF, init, args)` and nothing else: naming
   `init` there is what lets the splitter move the hack into its own chunk.
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

Thirty-eight of the 2D hacks came from xlockmore and are written against
`ModeInfo` and the `MI_*` accessors rather than `screenhack.h`. They go through
`runtime::xlockmore`, which is the same adaptor `hacks/xlockmore.c` is upstream:
build a `ModeInfo` in `init` with the colour scheme the hack `#define`s, then
translate `MI_WIDTH` to `mi.width` and so on. Everything else about the port is
the same.

## Pictures

About thirty of the hacks work on an image rather than drawing from nothing.
Upstream grabs the screen or a file from your pictures directory; in a browser
there is neither, so a saver takes its pictures from atproto:

```text
/screensaver/decayscreen?images=@overby.me    that account's own photographs
/screensaver/decayscreen?images=%23caturday   whatever anyone posts under the tag, live
/screensaver/decayscreen                      colour bars
```

This crate never fetches anything, which is what keeps it dependency-free and
testable without a browser. Instead `runtime::image` is a channel: a hack asks
for a picture, the host answers when it can, and if nothing is going to answer
the hack gets SMPTE colour bars, which is upstream's fallback too
(`utils/colorbars.c`). The native tests register no host, so every
image-consuming saver runs against the test card.

The host side lives in `../src/images.rs`, which explains why every route goes
through the posting account's own PDS rather than `cdn.bsky.app` (no CORS
headers there, so the canvas would be tainted and unreadable) and why hashtags
come off the Jetstream firehose rather than a search endpoint.

One thing to know when porting an image-consuming saver: hacks ask for their
picture in `init`, so whether a host is available has to arrive with
`StartArgs::with_image_host`, not be set afterwards.

A separate thing with a confusingly similar name: about a dozen hacks carry
pictures of their own, which are program data rather than something the viewer
supplies. The Matrix glyph sheet, the face on the flag, the test cards a
television tunes between. Upstream turns each into a C array at build time
(`images/gen/NAME_png.h`); here the files sit in `images/` exactly as upstream
ships them, arrive through `include_bytes!`, and are decoded by
`runtime::png`, which is a small PNG reader plus the DEFLATE underneath it.

That decoder only covers what upstream's own files are stored in, which turns
out to be every colour type at bit depths 1 through 8, no interlacing and no
sixteen-bit samples. It returns the colour and, separately, a depth-1 bitmap of
where the picture is opaque, because `Fb` has no alpha channel to put it in and
neither does X: a hack draws a sprite by clipping the colour through the
bitmap, which is what `image_data_to_pixmap` hands it upstream.

## Code splitting

Each saver is its own lazily-loaded wasm chunk, declared in
`../src/pages/savers.rs`. Two things about that are worth knowing before
changing it:

- **`#[component(lazy)]` cannot be used.** It hardcodes the split module name to
  `"lazy"`, so every lazy component in the app shares one chunk and opening any
  saver would download all of them. The `lazy_loader!` macro underneath it takes
  a module name, so each saver declares its own.
- **The chunk exports the saver's own `start`, which runs the hack.** See
  below for why returning data instead does not work. `wasm-split` 0.7.9 never
  emits a shared chunk (`build_split_chunks` computes an empty set), so anything
  reachable from two split modules but not from `main` is copied into *both*;
  keeping the runtime reachable from `main` is what stops that. Nothing in
  `savers.rs` may call `xscreensaver::all()` or `find()`, which name every
  saver's entry point.

The `split` cargo feature deliberately depends on `wasm-splitter` directly
rather than on `dioxus/wasm-split`: enabling the dioxus feature also makes the
router split every route, which is both more than we want and enough to crash
dioxus-cli 0.7.9's splitter (`Failed to find data symbol`, reproducible with no
screensaver code involved).

### The boundary has to run code, not return it

Measured on clean builds of the same source, three savers registered:

| Build | main.wasm | chunks |
|-|-|-|
| no savers at all | 935,996 (376,517 gz) | n/a |
| savers, one module (`just build-whole`) | 1,025,257 (412,894 gz) | n/a |
| savers, split (`just build`) | 1,014,604 (406,909 gz) | 9.4 / 12.3 / 21.5 KB |

Loading `/screensaver/munch` fetches `homepage_bg.wasm` and
`module_1_munch_body.wasm` and nothing else.

The first attempt at this did not work: each chunk's entry point returned a
`&'static SaverDef` holding a constructor pointer, and every hack stayed in the
main module while the chunks came out at ~485 bytes. The splitter follows real
calls out of the exported function, and nothing in the chunk *called* the hack:
the only caller was `Runner`, in main, through a function pointer. Turning the
boundary around, so the chunk exports the saver's own `start` which names its
constructor directly, is what moves the code. Keep it that way when adding
savers, and do not reintroduce a table that names entry points.

Note that the main module shrank by 10.6 KB while 43 KB moved out: the
difference is splitter overhead (the indirect-call table, the import stubs).
Per visit the split build is therefore about break-even at three savers. The
property that matters is the other one: a new saver now grows only its own
chunk.

`--debug-symbols false` applies either way: DWARF is ~90 KB gzipped and needs a
browser extension to read.

## Licence

The hacks carry a permissive MIT-style notice ("Permission to use, copy, modify,
distribute, and sell this software..."), which is why every ported file keeps
its original copyright header. The panel credits each saver's author and links
its upstream demo video.

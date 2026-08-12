# xscreensaver

Rust ports of the [XScreenSaver](https://www.jwz.org/xscreensaver/) hacks,
served at `overby.me/screensaver/<name>`.

Upstream ships 314 screensavers. They fall into three groups, each needing a
different runtime:

| Tier | Savers | Upstream | Runtime it needs | State |
|-|-|-|-|-|
| 2D | 143 | `hacks/*.c`, Xlib | software framebuffer + Xlib façade | done (143) |
| Shadertoy | 30 | `hacks/glx/glsl/*.glsl` | WebGL2 multi-pass runner | done (30) |
| OpenGL | 137 | `hacks/glx/*.c`, GL 1.x | immediate-mode emulation over WebGL2 | in progress (120) |

`webcollage` and `vidwhacker` are not portable: they scrape images off the live
web. `co____9` is `covid19` under a name that does not date it, generated from
the same source at build time, so it is counted once. `mismunch` was retired
upstream in version 5.08 and merged into `munch`, which since then draws either
kind depending on a resource; the name still has a configuration file of its
own, so it has a slug of its own here too, pointing at the same code with the
resource nailed down.

The 2D and Shadertoy tiers are finished. `bsod` is all thirty-nine of its
computers, each one a little program for the same command queue, and `m6502` is
a 6502 with an assembler. The OpenGL tier has started; see below.

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

There is a second font, and it is not made of pixels. `runtime::glutstroke` is
GLUT's Roman simplex, the vector font upstream carries in `glut_roman.h`: a
character is a few open polylines in a hundred-unit em and how far to move
along afterwards. Nothing about it is filled, so a saver that wants a solid
letter builds one along the lines itself, which is what `gltext` does with a
tube along every segment and a ball at every joint.

## Pictures that are meant to break

`runtime::jpeg` is a baseline JPEG encoder and decoder, for the one saver whose
subject is a damaged file. Upstream's `glitchpeg` corrupts the bytes of the
photograph on disk and shows what the system decoder makes of them; a picture
arrives here already decoded, with no file behind it, so one is made.

The decoder is written to survive what it is given. A Huffman code that is not
in the table decodes as a zero, a coefficient index that runs off the end of
its block is dropped, and a marker where a restart should be is stepped over,
which is what libjpeg does when it resynchronises. The restart markers are the
reason the effect works at all: they are byte-aligned and unmistakable, so a
decoder whose bits have gone out of step can find the next one and be right
again from there. Damage shows as a band, not as the end of the picture.

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

## A processor

`hacks2d::asm6502` is upstream's `asm6502.c`: a 6502 interpreter and a
two-pass assembler, both ports of the JavaScript behind 6502asm.com. It lives
next to `hacks2d::m6502`, the saver, because it is the only thing that uses it
and the whole point of the split chunks is that nobody else pays for it.

The programs it runs are the `.asm` files in `images/m6502/`, unchanged, and
they are assembled from that text every time one starts, exactly as upstream
does. Nothing is precompiled to bytes: the assembler is part of the saver and
takes about a millisecond, once every thirty seconds.

Several of its instructions are wrong. `RTI` pops one byte of return address
instead of two, `TXS` pushes X rather than setting the stack pointer, `CMP`
sets carry from `A + M > 0xff` instead of `A >= M`. They are kept exactly as
they are, because the thirty-three programs were written on that web page
against that interpreter, and an instruction repaired here is a program broken
there.

## A convex hull

`runtime::quickhull` is Karim Naaji's `3d-quickhull`, which upstream carries as
`quickhull.c`: the smallest shape that contains a cloud of points. `crumbler` is
what wants it, because its pieces are not modelled at all. A piece is a few
thousand random points and what you see is the hull of them, so breaking one in
two is a matter of splitting the points and taking two hulls.

Two things are done differently from the C. It allocates faces and edges for
`n * (n - 1)` triangles up front, which is gigabytes at the point counts
`crumbler` uses and only works because a system that overcommits never hands out
the pages; here they are vectors that grow to the few thousand actually used,
and the out-of-memory retry loop that reduces the density until the allocation
succeeds has nothing left to do. And its duplicate-point pass is a quadratic
scan that shuffles the array down on every removal, which is minutes of work at
the higher densities; the same rule is answered here with a grid of
epsilon-sized cells.

## The eighty uniform polyhedra

`runtime::kaleido` is Zvi Har'El's `kaleido`, which upstream carries as
`polyhedra.c`: a table of eighty Wythoff symbols, and the machinery that turns
one of those symbols into an actual solid. Nothing about it is a lookup. The
symbol says which spherical triangle to reflect in and how often the generating
point goes round each of its corners; from that the code recovers the rotation
group, solves for where the point has to sit with Newton's method, walks the
group to get every vertex, and reads the faces off the incidence structure.
The dual comes from the same data with vertices and faces swapped.

Two divergences. There is no edge list, since nothing asks for one, and no
name-guessing pass, since everything is asked for by table index rather than by
Wythoff symbol. And the Newton solve is bounded to a thousand iterations: it
converges in a handful, but a loop with no bound at all is a browser tab that
hangs rather than a program that can be killed.

The tests check each of the eighty against its published Euler characteristic
and density, that every vertex lands on the unit sphere, and that the incidence
matrix and the adjacency matrix agree with each other.

`runtime::teapot` is the hundred and sixty-first solid, which is a joke, and is
Martin Newell's teapot as thirty-two bicubic Bezier patches. Upstream hands the
control points to `glMap2f` and lets `glEvalMesh2` walk the grid with
`GL_AUTO_NORMAL`. There are no evaluators here, so the Bernstein polynomials
and their derivatives are worked out directly, which is what an evaluator does;
upstream's own OpenGL ES fallback instead ships a large table of triangles that
had been evaluated in advance, and evaluating them is both smaller and rounder.

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

## Shapes

A couple of dozen of the OpenGL savers are a program wrapped around a model
somebody drew: a toaster, a skull, a golden apple, the polyhedra `tronbit`
wears. Upstream converts each to C source at build time, a flat array of
interleaved floats plus a `struct gllist` header saying how to read it, and
draws it with one `glInterleavedArrays` and one `glDrawArrays`.

Here the arrays are assets rather than source, because a Rust file with tens of
thousands of float literals in it takes minutes to compile. `gen-gllist.nu`
converts one:

```console
$ nu gen-gllist.nu <checkout>/hacks/glx xscreensaver/models tronbit_no.c
tronbit_no: 1080 verts, n3f_v3f, triangles -> xscreensaver/models/tronbit_no.gllist
pub const TRONBIT_NO: &str = include_str!("../models/tronbit_no.gllist");
```

It keeps upstream's literals character for character and only strips the C
around them, so a converted model diffs cleanly against its source. Paste the
`const` it prints into `src/models.rs`; `runtime::gllist` reads it back and
replays it into the recorder, wireframe included.

Size is not the constraint it looks like. The cow in `bouncingcow` comes to
970 kB across its six parts and that is fine: a model is text rather than
source, so it costs nothing to compile, and it lands in one lazily-loaded
chunk that is only fetched by someone who opens that saver. What is worth
checking first is the *vertex* count, which is what gets drawn every frame: the
cow's hide is thirteen thousand and three cows are still cheap.

One thing to check on any saver that wraps a photograph round something:
**textures here are top-down and OpenGL's are bottom-up**. A texture
coordinate of zero is the *first* row of the decoded image, where in GL it is
the last, so `v` has to be turned over: `v = 1 - v_gl`. It does not show on a
texture that is symmetric or abstract, which is most of them, and it shows
loudly on one that is not. `peepers` wraps a photograph of an eyeball along a
lathe, white at the front and red at the back; with `v` the wrong way round
every eye comes out bloodshot with a pale pupil, which is wrong but not
obviously a bug.

Not every shape is drawn, though. `runtime::marching` is upstream's
`marching.c`: give it a function that says how solid any point in space is and
it walks a grid over it and builds the surface where that value crosses a
threshold. `lavalite` is what wants it, and it is why its blobs merge and part
rather than passing through each other. The cost is the *grid*, not the
geometry: it calls the field function once per grid point and six more times
per emitted vertex when smoothing, so the resolution knob is quadratic in what
it costs and its top end is set by measurement.

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

The Shadertoy savers were the open question: their chunks hold shader *text*
rather than code, and the splitter moves code. It moves the text too. Measured
on the full bundle, `starnest` (1.8 KB of GLSL) is a 9.8 KB chunk and `skyline`
(34.7 KB of GLSL) is a 43.4 KB one, so the thirty programs are not sitting in
the main module.

`--debug-symbols false` applies either way: DWARF is ~90 KB gzipped and needs a
browser extension to read.

## The Shadertoy tier

Thirty savers, and between them they are one C program: `hacks/glx/xshadertoy.c`
plus a `.glsl` file each. Upstream builds each one as a shell script
(`xshadertoy-compile.pl`) that runs `xshadertoy` with the GLSL on stdin, so a
saver *is* its shader plus a handful of knobs, and here it is a `ShadertoyDef`:
the sources as `include_str!`, and the knobs the XML declares, which are the
same five everywhere (`delay`, `speed`, `scale`, `showfps`, and `duration` for
the one saver with variants).

The tier is split across two crates, which is not how the 2D tier works and is
worth understanding. `xscreensaver::shadertoy` decides *what* to draw: which
variant is running, what the uniforms are this frame, and how to assemble a
pass's source. `../src/pages/gl.rs` owns the WebGL2 context and draws it. So the
half that has the logic in it is testable with no browser, the same bargain the
software framebuffer makes for the 2D savers, and the half that cannot be is
about a hundred lines of API calls.

What the runner does:

- Up to five passes (BufferA to D, then Image), each a fragment shader drawn
  over two triangles into its own texture at `size * scale`, plus an optional
  `Common` source textually prepended to all of them. The assembled source is
  the version line, a fixed preamble of the `iTime`/`iMouse`/`iChannelN`
  uniforms, the common source, `#line 0`, the saver's own source, and a `main`
  that calls its `mainImage`. Under GLSL ES 3.00, which is what WebGL2 is, the
  preamble's GLSL-1.20 compatibility half compiles out.
- Pass *i* binds every pass's latest texture to `iChannel0..3`, so a later pass
  reads an earlier one's output. Upstream's loop runs over five channels into a
  four-element array and reads past it; four is all a shader can declare.
- A saver may carry several variants, which are whole alternative programs; it
  steps to the next every `duration` seconds, recompiles, and starts its clock
  again. Only `bestill` has them, six of them.
- Time is warped by `speed`. `iMouse` is a four-value state machine rather than
  a position: `xy` is where the pointer is, `zw` is where the drag began and
  goes negative when the button comes up, so a program can tell a click from a
  drag from a release from never having been touched.
- `iChannelResolution`, `iChannelTime` and `iSampleRate` are declared and never
  set, exactly as upstream leaves them, and `iDate` gets a time of day but a
  zero date. Nothing in the collection reads any of them. Keyboard input is not
  supported upstream either; it wants a texture with a bit per key.

Three things had to be different, and each of them is the difference between a
picture and a black screen:

- **Every pass has two textures, not one.** Upstream binds a pass's own output
  to its own `iChannel0` while drawing into it, which is a feedback loop and
  undefined in OpenGL; what it does in practice is read the previous frame, and
  four of the programs are written expecting exactly that. WebGL2 refuses the
  draw instead. So a pass renders into its back texture and then makes it the
  front, which gives a pass the finished output of everything before it in the
  chain and last frame's output of itself. That is also what shadertoy.com does.
- **The canvas has to ask for `antialias: false`.** The frame ends by blitting
  the last pass onto the canvas, and blitting into a multisampled framebuffer is
  an error. A canvas is antialiased by default. Upstream has the same
  requirement and meets it with `*forceSingleSample: True`.
- **A texture has to be bound before it can be attached.** `createTexture`
  returns an object with no target, and attaching one fails with "no texture is
  bound to the specified target", so each is given a pixel at creation and its
  real size later.

A canvas has one context for its lifetime, 2D or WebGL2 and never both, so the
stage picks its engine from the saver's tier before it mounts. The per-saver
wasm chunk holds only its shader text, so the rule about a chunk having to *run*
code does not bite here: there is no per-saver code to strand in the main
module.

### Looking at one

`just shot` cannot render these: it runs the crate's software framebuffer, and a
fragment shader needs a GL driver. `test-browser.nu` serves the built bundle,
drives headless chromium at it over the DevTools protocol, and saves a PNG each:

```sh
just build-whole
nu test-browser.nu starnest
nu test-browser.nu --all                     # all thirty, plus a montage
nu test-browser.nu skyline --console --query "scale=0.25"
```

It waits for the stage to mount before it starts timing, which matters: a
screenshot taken while the wasm is still arriving is the browser's default
white, and that is indistinguishable from a saver that drew nothing. The
earlier approach, `chromium --screenshot` with `--virtual-time-budget`, is worse
than useless here — it starves the animation loop, so it reports a black canvas
for shaders that render fine, and a *different* set of them each run.

## The OpenGL tier

The 136 remaining savers are written against OpenGL 1.3: a matrix stack, a
fixed-function pipeline, and `glBegin`/`glVertex`/`glEnd`. None of that survived
into OpenGL ES 2, so none of it is in WebGL. Upstream hit this first, porting to
iOS and Android, and answered it with `jwzgles.c`, which implements the old
calls in terms of the new. `runtime::gl` is the same answer with one change:
`jwzgles.c` *is* a GL binding and makes GL calls as it goes, and this records
instead. A frame comes out as a `Frame` of vertices plus the batches that say
what to draw and under which matrix, and `../src/pages/gl3d.rs` hands that to
WebGL2.

The indirection buys two things. The savers stay testable with no browser and
no GPU, which is what the whole 2D tier is built on: `hacks3d`'s tests check
that a saver emits geometry, that the geometry lands inside the clip volume,
that it moves, and that it is the same for the same seed. And the batching is
free, because `glBegin`/`glEnd` around three vertices is an append to a `Vec`
here rather than the driver round trip it used to be.

What is implemented so far is what the ported savers need, and it grows with
them: the matrix stack with `glRotate`/`glTranslate`/`glScale`/`glFrustum`,
`gluPerspective` and `gluLookAt`, `glBegin` through `glEnd` for every primitive
(quads and polygons are cut into triangles as the block closes, since ES has no
such thing), per-vertex colour and normals, point size, the depth test and a
mid-frame clear of it, face culling and winding, blending, display lists, two
lights, fog and texturing.

Textures are RGBA bytes and nothing else: no mipmaps, no other formats, and
always `GL_LINEAR`, with a choice of repeating or clamping at the edges,
because that is what every saver that makes one asks for. A texture that is one
picture rather than a tile clamps, and `rubikblocks`, whose faces each carry a
dark outline, is the one that cares. They are built once when a saver starts
and referred to by
name after that, so the host uploads each one the first time it sees the name
and keeps it. A texture carries a generation number for the savers that do not
work that way: `cubenetic` rebuilds sixty-five thousand pixels of interference
pattern every frame, and the counter is how the host knows to upload it again.
There is no `GL_TEXTURE_1D` in WebGL, so a saver that wants one gets a 2D
texture a single row high, which samples the same way. `quasicrystal`, whose
entire picture is one row of sine repeated a few hundred times across seventeen
quads, is the reason it exists.

Blending is an enum of the source and destination pairs the savers actually
pass, rather than the full cross product, because they pass six of them between
the lot of them. `lockward` is the reason for one of the odd ones out,
`GL_DST_COLOR, GL_SRC_ALPHA`: it multiplies what is already on the screen, so a
flash lights up the spinner and leaves the black around it alone.

One thing cannot be expressed at all: the logic ops, `glLogicOp`, which do
bitwise arithmetic on the framebuffer. WebGL has no equivalent, and the only
saver here that reaches for one, `quasicrystal`, wants `GL_AND_REVERSE` to
raise its contrast. What that does arithmetically is invert what is there and
scale it, and that *is* expressible as a blend, so the port uses one and says
so. If a saver ever wants a logic op that has no arithmetic reading, it will
need a render target and a second pass.

Fog matters more than it sounds for anything running off towards a horizon.
`gravitywell`'s grid is five hundred units across, and without fog every line
at the far end is drawn as brightly as the ones in front and the back half is a
solid band.

The texture environment has both modes too. `GL_MODULATE` is the default and
means the texture multiplies the colour under it; `GL_ADD` means the colours add
and only the alphas multiply, and `energystream` wants it so its flares pile up
into white where they overlap rather than darkening each other.

`GL_ALPHA_TEST` is there too, in the one comparison the savers use
(`GL_GEQUAL`): a fragment whose alpha comes out below the reference is
discarded rather than blended. The distinction matters whenever a texture is a
cut-out. `glforestfire` stands its trees on quads whose background is
transparent, and blending them would leave depth written across the whole quad,
so the sky behind a tree would come out as a rectangle of nothing. Discarding
writes neither colour nor depth. Fog also runs in two modes: `GL_EXP2`, where
what survives at a distance is `exp(-(density * distance)^2)`, and `GL_LINEAR`,
which ramps between two distances.

The stencil buffer is there in the cut-down form the savers use it in: a
comparison of `GL_ALWAYS`, `GL_EQUAL` or `GL_NOTEQUAL`, a reference value, and
what a passing fragment does to the buffer, which is one of `GL_KEEP`,
`GL_REPLACE` and `GL_INCR`. The two chess savers want two things from it. One
is a mirror with an edge: `queens` paints its board tiles into the stencil with
the colour mask off, then draws the pieces a second time upside down under the
board with the test set to `GL_EQUAL`, so the reflection lands on the tiles and
stops dead where they do. The other is shadows: `endgame` flattens every piece
onto the board through a projection matrix built from the light's position,
marks the silhouettes into the stencil rather than drawing them, and then
washes one dark quad over the whole board wherever the mark is not zero.
Drawing the flattened pieces directly would darken the board twice where two
shadows crossed; marking and washing gives their union. Anything more elaborate
than that (separate front and back faces, decrement, write masks) would need
the state to grow, and nothing has asked.

Lights attenuate with distance, `1 / (c + l*d + q*d*d)` as OpenGL has it, which
costs nothing for the savers that leave the terms at their defaults and turn
into one. `endgame` is the one that needs it, and not for falloff: it fades a
whole game in and out by winding the constant term from a hundred down to one,
which is dark to fully lit and back.

`runtime::tess` is the polygon tessellator, standing in for GLU's. It is ear
clipping: find a corner whose triangle sticks out of the outline and holds none
of the other corners, cut it off, repeat. That is enough for a simple polygon
and not enough for one that crosses itself, which GLU would fill by the odd
winding rule; nothing has needed that yet.

For a long time there was none, because upstream's own OpenGL ES build has no
GLU either and every saver that wants one already carries a fallback: `jigsaw`
cuts each puzzle piece into eighths and fans each one out by hand, and
`dnalogo` carries the triangles its desktop build's tessellator produced for
the pizza slice once and had saved. Both ports still take that path.
`polyhedra` is the one with no fallback to take: several of the duals have
faces shaped like three-pointed stars, and fanning one of those out from a
corner fills in the notches.

There is no texture *generation* beyond `GL_SPHERE_MAP`, and no texture
matrix. `atlantis` is the saver that wants more: it lays a noise texture over
the whole tank with `GL_EYE_LINEAR`, which reads the texture at a fragment's
own position in eye space, and scales it down with a matrix. Upstream's own
OpenGL ES build has neither and quietly drops the effect; the port works the
coordinates out per vertex instead, which is what the fixed-function pipeline
would have done, and folds the scale into the planes.

One thing is recorded and then thrown away: line width. WebGL draws every line
one pixel wide whatever it is asked for, on every implementation that matters,
so a saver that wants a four-pixel line gets a one-pixel one. `boing` is the
one where it shows, in its grid and its scanlines. Widening lines properly means
expanding each segment into a quad in the host, which is a change worth making
when a saver depends on it more than these do.

The one cost this design carries is that a display list is *replayed as
geometry* on every `glCallList` rather than living on the GPU, and every matrix
change starts a new batch. Both totals therefore scale with the number of
objects a frame draws, not with the number of lists it compiled. For
calibration, at 1280x720 `splodesic` draws 5120 batches, `cubestorm` 2880
batches and 34k vertices, and `beats` 343k vertices, and all three are fine; six
thousand batches is about the ceiling. Three savers are deferred on this rather
than on anything about their code: `glcells` at its default quality draws 800
cells of 640 triangles each, and `squirtorus` about thirty thousand quads per
sphincter with sixteen on screen. `covid19` draws sixty virus particles whose
spikes and surface proteins come to some seven hundred matrix changes each, so
around forty thousand batches a frame. Measure before starting one of the
crowded ones.

`deepstars` was on that list and came off it, which is the shape to look for
before giving up on a crowded saver. Its star trails are the whole sky redrawn
up to four hundred times a frame, each copy turned a little further and drawn
fainter: eight million points and a hundred and fifty thousand draw calls.
But the copies differ only by a rotation and an alpha, and folding the rotation
into the vertices leaves the alpha as the only difference — and an alpha is
vertex data, not batch state. The whole exposure of one colour then goes down
in a single call: 97 batches at any setting, from 151009. What still had to
give was the length of the exposure, not the number of stars; a sparser sky
with longer trails was tried first and looked like a different picture, since
how dense the sky is is most of what it looks like.

`extrusion` is deferred on a dependency rather than on anything about its own
552 lines: it draws everything through GLE, the tubing and extrusion library,
which upstream does not bundle and links against. Porting it means writing
`gleExtrusion`, `gleTwistExtrusion`, `gleSuperExtrusion`, `gleHelicoid`,
`gleScrew` and `gleTaper` first, with their join styles, which is a library in
its own right.

`timetunnel` is deferred for a different reason: the two functions that draw
its signs and its tunnels are wrapped in `#ifndef HAVE_JWZGLES` upstream, so
its own OpenGL ES build draws neither. They want a blend *constant*
(`glBlendColor`), a blend *equation* (`GL_FUNC_REVERSE_SUBTRACT`) and a texture
matrix, none of which this runtime has either. Porting it faithfully means
adding all three first; porting it as upstream's mobile build behaves means
shipping a saver with most of itself missing.

A saver whose crowd is a *setting* can keep its code and lower the setting
instead. `winduprobot` draws twenty-five robots of sixty-three thousand
vertices each, which is 1.6 million a frame; its default here is five, which
comes to the same 334k `beats` draws, and its slider still goes to a hundred.
Say so in the saver's own comment when you do this, with the measurement.

`hopffibration` is the one where lowering the setting is not enough, and it is
worth knowing why, because the ceiling it hits is vertices rather than batches.
Its fibers are tubes swept along curves that are subdivided until they are
smooth, and its heaviest animation draws two hundred and sixteen of them. In
the coarsest of the three detail levels its own knob offers, that is 767k
vertices a frame, or 35 MB written and uploaded thirty times a second; at the
default it is 1.6 million and 74 MB. The batch count is fine, because the fiber
colours ride on the vertices and the whole thing goes down in one call. What is
not fine is that upstream sends the geometry to the card once and draws it from
there many times over, and a recorder that rebuilds every vertex of every frame
has no equivalent. The maths, the base-point choreography and the eight-by-eight
table of animations are all ported and tested in `runtime::hopf`, with the
measurement as a test; what is missing is a way for geometry to outlive a frame,
which would also lift `covid19`, `glcells` and `squirtorus`.

Batches also break on *state*, and the state that catches people out is the
front-face winding. Two quads with opposite windings can never share a batch, so
a loop that emits a top face and then a bottom face, over and over, costs two
batches per iteration however few vertices it moves: the spoke drawing in
`runtime::involute` cost 381 batches for one gear that way, and `geodesicgears`
draws ninety-two gears from one shape. Walking the loop once per winding draws
the same quads in a different order and comes to four batches, which is what it
does now. The gears are opaque and depth-tested, so nothing about the picture
depends on which order they went down in.

One shape of expensive frame has a way out. A saver that billboards sprites
takes the modelview matrix, forces its rotation to the identity and loads that
back, once per sprite, which is a batch each. What that comes to is a quad
standing square to the camera at the sprite's transformed position, so
transforming the positions in the port and emitting the lot against an identity
modelview draws the same pixels in one batch. `dumpsterfire` does this with ten
thousand of them.

The same reasoning covers the savers built out of `tube`, which puts each
cylinder where it goes by pushing a matrix. `runtime::tube::TubeMesh` builds the
unit cylinder once, flattens it into a triangle list a strip or a fan could not
be concatenated into, and transforms it into place on the way out, so every tube
drawn between two matrix changes lands in one draw call. It took `highvoltage`,
which traces each of its towers out of some six hundred tubes, from 6269 batches
a frame to 20.

A depth clear partway through a frame is an ordering, not a state, so it rides
on the batch it precedes rather than on the frame. `voronoi` is the reason:
it draws each site as a cone and lets the depth buffer work out which region
belongs to whom, which fills the depth range, so the markers showing where the
sites are would be buried inside the cones without a clear before them.

Blending is the `glBlendFunc` pairs the savers pass rather than the full matrix
of factors, and each new pair becomes another variant. Two of the ported savers
are mostly made of it: `hypnowheel` is `GL_ONE, GL_ONE`, a stack of translucent
spirals whose overlaps add up towards white, and `cubestack` is
`GL_SRC_ALPHA, GL_ONE` with the depth test off, so every cube in its stack shows
through every other.

`glDepthMask` and `glPolygonOffset` are both per batch. The first is what lets
a translucent object blend with itself rather than have its nearest face hide
the rest, which is how `engine` shows its machinery through the block; the
second is for two surfaces drawn in the same place where one has to win, which
is how `geodesic` overlaps two frequencies of the same sphere while one fades
into the other. `glColorMask` and the three `glDepthFunc` modes the savers ask
for are there too: `molecule` draws its electron shells once with the colour
mask off, purely to fill the depth buffer, and again where the depth is exactly
equal, which is what stops a translucent shell piling up on itself.

`runtime::easing` is upstream's `utils/easing.c`, all thirty-one curves, which
nineteen of the savers use. They are CSS's semantics and CSS's constants, so
the springy ones genuinely overshoot: `back` pulls away before it sets off and
`elastic` rings around the end before it settles.

There are two lights, because two is the most any saver here turns on, and the
limit grows when one wants more. A light's position goes through the modelview
matrix as `glLightfv` is called, which is what fixes it to the scene rather than
to the object about to be rotated, and it arrives at the shader already in eye
space, `w` and all: a `w` of zero means a direction, and any other means a
homogeneous point that has to be divided through before the direction to it
means anything. `menger` passes `w` of 0.1, so that division is not academic.
Two deliberate differences from OpenGL 1.3, neither of which changes what is
depicted: the shading is per fragment rather than per vertex, which on shapes
this low-polygon only looks better, and lighting is two-sided, so the savers
that leave culling off can see the inside of a thing as well as the outside.

There are no spotlights: no cutoff, no exponent, no attenuation. Only two
savers want one, and both can have it without the runtime growing. `circuit`
approximates its with a point light, because all its does is pick out the
component nearest the front. `antspotlight` cannot, since its beam *is* the
picture: the floor is unlit and the only part of the image ever drawn is a fan
of triangle strips spreading out from under the ant. That fan is built afresh
every frame, so the port works the light out per vertex and hands it over as a
vertex colour. That is not an approximation of `GL_SPOT_CUTOFF`; it is the same
arithmetic, since fixed-function lighting is per vertex too, moved out of the
pipeline and into the saver. Any saver whose lit geometry it generates itself
can do the same.

Materials are `GL_AMBIENT_AND_DIFFUSE` in one field, because that is what the
savers set; with lighting on, vertex colours are ignored, which is OpenGL's own
rule rather than a shortcut. The one thing a material carries twice is that
colour, once for each face, since a few savers turn culling off and paint the
inside of a surface differently from the outside: `splodesic`'s sphere is one
colour outside and the opposite colour in, which is most of what it looks like
once it starts coming apart. Setting the front colour sets both, so a saver only
has to say anything if it wants them to differ.

State that can differ between one `glBegin` block and the next rides on the
batch rather than on the frame, which is what lets a saver turn the depth test
off for one thing and leave it on for another.

`glMaterial` is one of the few calls OpenGL allows *inside* a block, and savers
use it: `cityflow` draws eight hundred boxes as one long run of quads and
changes the colour between each of them. A batch carries one material, so
setting one cuts the run in two and carries on. Only where the vertices are
independent primitives and only on a primitive boundary, since cutting a strip
or a fan in half would lose the triangles that straddle the cut.

Runs of blocks with nothing between them but more vertices are folded into one
batch, which matters more than it sounds: a saver drawing a cube as forty-eight
separate quads is forty-eight `glBegin` blocks, and `cubestorm` draws eight
hundred such cubes a frame. Folding turns thirty-eight thousand draw calls into
eight hundred. It is only done for points, lines and triangles, because joining
two triangle strips end to end is not one longer strip: the seam would grow a
pair of triangles nobody asked for. `hexstrut` needs exactly that:
its sheet is flat and its struts overlap, so upstream turns depth testing off
and lets them stack in the order they were drawn.

`runtime::shapes` is upstream's `sphere.c` and `normals.c`: a unit sphere,
which twenty-seven of the savers draw, and the normal of a triangle, which
thirty-one of them compute. The sphere is one long triangle strip rather than a
strip per band, which is upstream's shape and worth keeping: the join between
two bands is a pair of degenerate triangles that cover no pixels and turn the
whole thing into a single run.

`runtime::tube` is `tube.c`: a cylinder or a cone between two points, which
seventeen savers want. The geometry is only a unit tube about the y axis; the
interesting part is that it aims one with two rotations rather than a basis,
and that the cap size extends a tube past both of its ends, so a chain of them
meeting at angles reads as one continuous bent pipe with no notches at the
joins. `glknots` is eight hundred of them end to end.

Two more helpers came with the tier because nearly every saver in it uses them,
and they are in `runtime::rotator`: the `rotator`, which is what turns an object
over on its own without ever quite repeating, and the `gltrackball`, which is
the SGI virtual trackball from 1993, deforming from a sphere into a hyperbolic
sheet away from the centre so that a drag near the edge spins rather than
tumbles.

A display list records commands rather than results, which matters: `glCallList`
runs it under whatever matrix is current at the time, so a saver that compiles
one lattice and draws it from a new angle every frame, which is exactly what
`cubicgrid` does, gets a new picture out of the same list.

These cannot be rendered with `just shot` either. Use `test-browser.nu`, as for
the Shadertoy tier.

## Licence

The hacks carry a permissive MIT-style notice ("Permission to use, copy, modify,
distribute, and sell this software..."), which is why every ported file keeps
its original copyright header. The panel credits each saver's author and links
its upstream demo video.

# xscreensaver

Rust ports of the [XScreenSaver](https://www.jwz.org/xscreensaver/) hacks,
served at `overby.me/screensaver/<name>`.

Upstream ships 314 screensavers. They fall into three groups, each needing a
different runtime:

| Tier | Savers | Upstream | Runtime it needs | State |
|-|-|-|-|-|
| 2D | 145 | `hacks/*.c`, Xlib | software framebuffer + Xlib façade | done (145) |
| Shadertoy | 30 | `hacks/glx/glsl/*.glsl` | WebGL2 multi-pass runner | done (30) |
| OpenGL | 138 | `hacks/glx/*.c`, GL 1.x | immediate-mode emulation over WebGL2 | in progress (137) |

`webcollage` and `vidwhacker` are neither: upstream is a perl script and a C
helper for the first and a shell script for the second, rather than a
`hacks/*.c`. What they need at run time is a framebuffer, so they are counted
with the 2D tier. `co____9` is `covid19` under a name that does not date it,
generated from the same source at build time, so it is counted once.
`mismunch` was retired upstream in version 5.08 and merged into `munch`, which
since then draws either kind depending on a resource; the name still has a
configuration file of its own, so it has a slug of its own here too, pointing
at the same code with the resource nailed down.

The 2D and Shadertoy tiers are finished. `bsod` is all thirty-nine of its
computers, each one a little program for the same command queue, and `m6502` is
a 6502 with an assembler. One of the OpenGL tier is left, `worldpieces`,
waiting on a triangulator rather than on anything about the Earth; it is
described at the end.

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

The host fills it the way it fills the picture channel, chosen with a `text`
parameter: `?text=@handle` reads an account's posts, `?text=%23tag` reads a
hashtag live, and `?text=<url>` reads any URL you name. With nothing set the
words are a poem, which is the nearest thing to `fortune(6)` still answering:
`fortune` has no surviving public API, and the quote services that do answer
either send no `access-control-allow-origin` or return a single line, which is
thin material for a saver that wants a stream.

Two things about that channel are worth stating because getting either wrong
is invisible until you look. Text from the host is folded to the width the
hack asked for, exactly as the compiled-in passage is, because several hacks
lay the words out as they arrive and would otherwise run a paragraph off the
side of the screen. And a host that promises words and then fails gets twenty
seconds before the passage is served instead: without that, a source that does
not answer leaves the screen permanently blank, which is worse than the wrong
words. The picture channel already fell back to colour bars; this one did not
fall back at all.

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

One trap when a test reaches past `start` and builds a hack's own structs: the
random generator has to be seeded first, with `ya_rand_init`. `runtime::rand`
is a faithful port of upstream's `yarandom.c`, including that its unseeded
state has both lags at index zero, which makes every call double one element in
place; after about 1760 calls the whole vector has been shifted out to zero and
`random()` returns nothing but zeroes for ever. Upstream never notices because
it seeds before it draws, and neither does anything here that goes through
`Runner::start`. What it looks like when you do notice is a hang rather than a
wrong picture: `pipes` picks its starting cell by rejection sampling, and with a
generator that has stopped generating there is nothing left to reject to.

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

One set of those is not upstream's file byte for byte, and it is worth saying
which and why. The four maps of the Earth (`earth.png`, `earth_night.png`,
`earth_water.png`) are shipped at 4096x2048, which is 6.8 MB for the first of
them alone, and six savers want one or more. Every saver here arrives as its
own lazily-loaded chunk and `wasm-split` emits no shared chunk, so a file
referenced by six savers is carried six times over and, worse, is downloaded in
full by anyone who visits one of them. They are stored at 1024x512 instead: a
quarter of the size in each direction, 515 KB for the day map. That is not a
judgement call about how much detail a globe needs, because upstream already
made it. `dymaxionmap` halves the built-in maps until they are under 2048
wide before it uses them, with the comment "the 2048x1024 images kill
performance", and a globe drawn 500 pixels across samples about one texel per
pixel at this size anyway. `earth_flat.png` is upstream's own file untouched,
because it ships at 1024x512 already.

The same reasoning applies to `klondike`'s fifty-two cards, which upstream
renders from SVG at 360x540 and pads with a drop shadow, coming to three
megabytes. They go in at half that size with a thirty-two colour palette,
which is 392 KB and is indistinguishable at the size a card is ever drawn:
the face cards are line art, and quantising line art costs nothing. Their
own licences are in `images/klondike/attribution.txt`, which is upstream's
file: the fronts are public domain and the back is CC BY-SA.

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

### The other model format

One saver predates `gllist` and has its own. `pipes` bolts nine shapes onto its
plumbing, and those came out of Lightwave 3D in 1997 as three flat arrays each:
the points, one normal per polygon, and a stream of polygon records. A record is
a vertex count, that many point indices, and one filler slot; a count of nought
ends the stream. Nothing indexes the normals, which are read in order, one per
polygon, so the thing has to be walked rather than handed to `glDrawArrays`.
`buildlwo.c` is the ninety-eight lines that walk it.

That is all "blocked on LWO models" ever meant. `gen-lwo.nu` converts the file
the same way `gen-gllist.nu` does, and `runtime::lwo` is the walker:

```console
$ nu gen-lwo.nu <checkout>/hacks/glx xscreensaver/models pipeobjs.c
BigValve: 716 points, 3785 polygon words -> xscreensaver/models/pipes_bigvalve.lwo
...
```

The one departure is that a face is drawn as a fan of triangles rather than as a
`GL_POLYGON`. The recorder can only take a polygon as a triangle fan, and a fan
cannot be merged with the fan beside it, so six hundred and twenty faces on one
valve would be six hundred and twenty draw calls. The faces are planar and
convex and the normal is flat, so splaying each one from its first vertex draws
exactly the same triangles in one batch.

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
thousand batches is about the ceiling. Three savers were deferred on this rather
than on anything about their code, and re-measuring all three is what turned up
the joining trick above and the covid19 correction below. None of them is
blocked; what each needs is written down here so the next pass does not have to
derive it again.

`covid19` is done, and it is the cautionary one: see below.

`glcells` is done, and it is where that lowering was applied. A cell is a
half-dodecahedron subdivided `quality` times; upstream's quality is 3, which is
10 * 4^3 = 640 triangles a cell and 1.54 million vertices for a full colony of
800. One step down is 160 triangles and 384k, which is comfortable, and quality
is an internal constant that upstream's own configuration file does not expose.
The colony is what the saver is about rather than how round any one cell is, so
that is the setting lowered, and the knob is offered so it can be put back. The
cells all share one shape and differ only by a matrix, so the matrix is applied
on the way out and the whole colony is one draw call however many there are.

Its subdivision is done differently from upstream's, which splits every triangle
into four and then welds the duplicates back together by searching the whole
vertex list. Remembering each edge's midpoint as it is made gives the same
shape without the search. The test is Euler's formula: the base is a
half-dodecahedron, which is a disc rather than a closed surface, so V - E + F is
one and stays one however often it is subdivided, and a weld that missed an edge
would break it.

`squirtorus` fires rings out of sixteen sphincters, and was deferred on the
worry that all of them might be in flight at once. Measuring first was the whole
of it, and it moved the answer twice.

The hole, not the ring, is the expensive part. Upstream compiles a hundred
display lists for it at a hundred degrees of openness, which is impossible here
where a list is replayed as geometry: one hole is 109,500 vertices, so a hundred
of them is eleven million. But a hole is a surface of revolution, so keeping the
150-point profile and generating the mesh at draw time costs nothing in memory
and nothing per frame either.

That leaves how many holes to draw, and the number to measure is not the vertex
count of a hole but how often a burst happens. Over twenty thousand frames, at
upstream's sixteen a ring is in the air 81% of the time and there are 23 of them
when there are any; at six it is 45% and 8.9. What decides it is the worst
frame rather than the average, because a burst puts twenty toruses in the sky
at once: over four thousand frames, six peaks at 812 thousand vertices and ten
already peaks at 1.57 million. Six is the largest count whose worst frame still
fits, so six is the default, and the knob still goes to fifty.

Its one porting trap has nothing to do with any of that. The stars are drawn
under an orthographic projection which upstream pushes and pops around them;
leaving it set means every frame after the first draws the world with it, and
the whole scene lands past the far plane. What that looks like is a starfield on
an empty black page, which reads as "the geometry is wrong" and is not. Probing
one vertex through `batch.mvp` said where it actually landed in a second.

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

`vidwhacker` was the last one on the not-portable list, and it is the one whose
label was most misleading. It reads a video capture card, which nothing here
can do, so it went in the bin with the others. But almost none of the program
is about that: it is a shell script that grabs a frame and pipes it through one
of nineteen netpbm pipelines, and the pipelines are the saver. Reading the
frame is two lines of it.

So porting it meant porting netpbm. `runtime::netpbm` is `pamedge`, `pamoil`,
`pgmbentley`, `ppmrelief`, `ppmspread`, `ppmshift`, `pgmenhance`, `pnmnorm`,
`pnmsmooth`, `pnminvert`, `pamarith`, `pgmnoise`, `pamcrater` and three of
`ppmpat`'s patterns, from netpbm's own C. Four of the names in the script are
the old ones and searching for them finds nothing: `pgmedge` is `pamedge` now,
`pgmcrater` is `pamcrater`, `ppmnorm` is `pnmnorm`, `pnmarith` is `pamarith`.

Doing it from the source rather than from a description was worth it twice.
`ppm_luminosity` rounds rather than truncating, and truncating turns some greys
one shade darker on the way through `ppmtopgm`, because the three weights only
sum to one to within a float. And `pamarith` works in normalised samples, so a
multiply is a multiply of *fractions*: the byte arithmetic that is easy to
write instead would saturate everything to white rather than darkening it.

One tool had to be made faster to be usable. `pamoil` takes the commonest value
in a seven by seven window, and netpbm clears and then searches a 256-entry
histogram for every pixel of every plane, when at most 49 of those entries can
be non-zero. Touching only the entries that were used, and doing one plane
rather than three when the picture is grey (which both pipelines that reach it
guarantee, since they run `ppmtopgm` first), takes the oil pipeline at 1280 by
800 from 1.7 seconds to 0.18 and the worst of the nineteen from 1.9 to 0.49.
The output is unchanged, but only because the tie-break was kept: netpbm scans
its histogram in value order, so two equally common values resolve to the
darker one, and a running best would resolve to whichever the window happened
to be walked over first. That is the kind of difference that does not show up
in a screenshot.

Pipeline 10 is transcribed with a hole in it. It computes a second difference
into `FILE3` and then outputs `FILE1`, so the whole second half of it is dead.
That is upstream's, and the note is here so the next reader does not think the
port dropped something.

`webcollage` was on the not-portable list too, and for a better reason than
`sonar` was: it finds its pictures by feeding random words to image search
engines and pulling the results out of the pages that come back. None of that
can happen from a page. But that is the part of the program that answers where
the next picture comes from, and `runtime::image` is already a channel with a
host on the other end of it: the pictures come from atproto, so
`?images=@handle` collages an account and `?images=%23tag` collages a hashtag
live as people post to it. Upstream's description is "this is what the Internet
looks like"; the internet moved and the saver did not have to.

What is ported exactly is the arithmetic, because it is the whole look of the
thing. A picture is not scaled to fit: it is halved until it fits, which gives
a wide spread of sizes rather than a uniform one, and the rectangle it is
fitting into is itself halved with probability 0.3, twice. The chance of
cropping starts at 0.2 and climbs with size, and climbs by 0.7 for anything
banner-shaped, which saturates it. Crops are bell-distributed towards the
middle of the picture unless it is a banner, in which case they are uniform.
Paste positions deliberately hang pictures off the edges and are cropped back.
And every picture gets a sinusoidal alpha ramp around its border, without which
the collage is a grid of hard rectangles.

The one thing the framebuffer cannot do is upstream's alpha channel, so the
bevel is folded into the per-pixel blend instead of being written into the
image and composited afterwards. Same arithmetic, one pass.

`extrusion` came off the blocked list the same way `unicrud` did, by asking
what the saver needs rather than what it links against. It draws everything
through GLE, the tubing and extrusion library, which XScreenSaver does not
bundle, and porting GLE would be a library's worth of work. But the slice of
it the seven shapes use is one sweep with one join style: the only
`gleSetJoinStyle` any of them asks for is `TUBE_JN_ANGLE`, so none of the cut,
round or raw join machinery is wanted, and the named shapes are wrappers.
`gleHelicoid` is `gleSpiral` with a circular outline, and `gleSpiral` builds a
helical path with a transform per station and hands it to the ordinary
extrusion. So `runtime::extrude` is that one sweep, and every shape here is an
outline, a path, and sometimes a transform.

The interesting part is the corner. A swept shape is not a stack of copies of
its outline: at each station the outline sits in the plane bisecting the angle
between the segment arriving and the one leaving, so consecutive segments meet
along one shared ring, with no gap outside the bend and no overlap inside.
That ring is found by running a line through each outline point parallel to the
segment and intersecting it with the bisecting plane. The test for it asserts
exactly that sharing: the back ring of one segment and the front ring of the
next are the same points.

`unicrud` came off the blocked list by asking a different question. It picks a
codepoint anywhere from 0 to 0x2F800 and draws it four inches high, and
`runtime::font` is one compiled-in bitmap font of Latin glyphs, so it went in
the bin. But that is only a blocker if the *crate* has to own the font. The
saver draws its character as a texture on a quad rather than as an outline it
manipulates, and the host is a browser, which has fonts: so `runtime::glyph` is
another channel and the browser draws the codepoint. The coverage that gives is
better than upstream's, not worse, since upstream can only use the fonts the X
server was configured with.

Telling a character the browser lacks from one it has needs care, because every
browser draws something for a missing glyph rather than nothing. The test is to
render a codepoint in a private use area, which is unassigned by definition and
so always comes out as the missing-glyph box, and reject anything that draws
identically to it. Upstream does the same test one step later: it draws the
character and picks again if the result is blank. That path matters more than
it sounds, because most of the range is unassigned.

Its block table is carried across with a bug in it. Upstream lists `Tags` at
0xE0020 after an `Unassigned` at 0xE0080, which is out of order, and upstream's
own runtime check for exactly that never fires because the loop stops at the
block holding the character and characters stop at 0x2F800. The disorder is
left where it is and the test asserts the table is sorted only up to that
ceiling, plus that the one bad pair is still above it, so this starts failing
if a later table moves it down into range.

The character's name is the one thing that cannot be had. Upstream shells out
to perl for it and notes that the alternative is embedding the 943 KB of
`NamesList.txt`; the line is left empty, which is exactly what upstream prints
when its lookup fails.

`mapscroller` was on the not-portable list for the worst reason of the lot:
upstream's own comment. It forks a perl helper to fetch tiles and says "doing
https from C code is untenable... this program won't work on iOS or Android",
and that was taken at face value. But a browser is the one place where that
sentence is false, and the only real question was whether the tiles can be read
cross-origin. They can: openstreetmap.org sends
`access-control-allow-origin: *`. The whole helper, cache and all, reduces to a
`fetch`, because the browser already has an HTTP cache.

Its tiles needed a channel of their own. `runtime::image` answers "give me a
picture" one at a time, which is all any other saver wanted; this one wants
thirty specific images at once, each named by where it goes, arriving in
whatever order the network manages, while it keeps scrolling. So
`runtime::tiles` is the same idea with a key on it.

The sea map is the part worth reading about. Upstream carries
`oceantiles_12.png`, one two-bit pixel for each of 4096 by 4096 level-12 tiles,
blue where there is open water, and calls `XGetPixel` on it to avoid starting
in the middle of the Pacific and to turn around on reaching it. Decoded the
ordinary way that image is 67 MB of framebuffer, eight times the largest
picture this port decodes for anything else, to answer a yes-or-no question. So
`runtime::png::decode_mask` stops at the defiltered scanlines, where a two-bit
image is only four megabytes, and returns two megabytes of bits.

The tempting shortcut was to skip the map: the cities table is already here for
the caption, so call a position "sea" when the nearest city is far enough away.
Measured against that table, land reaches 1,273 km from a city (central
Australia) and sea comes as close as 1,072 km (the Coral Sea). The ranges
overlap, so no threshold exists, and the map has to be carried.

One city in the table is in the sea, and the data is right: Funafuti is an
atoll narrower than a level-12 tile. Upstream copes by giving up after a
thousand tries, and so does this.

`sonar` was on the not-portable list for years of this document's life and
should not have been, which is worth writing down as a way of being wrong. It
pings the hosts on your network and plots them by response time, and a browser
cannot open a raw socket, so it went in the same bin as `webcollage`. But the
ping sensor is one of two: upstream also has a 112-line simulation that makes
two teams of blips up, and that is what runs on any machine where the binary is
not setuid. The saver is 1,265 lines of scope and 112 lines of sensor, and only
the sensor was ever blocked.

Choosing a ping option here therefore does what choosing it upstream on an
unprivileged machine does: the reason is shown for six seconds and the
simulation runs instead. That is not a stub standing in for the real thing, it
is the real thing's own failure path, which is why the message is worded like
the ones in `sonar-icmp.c`.

Its sweep is the one place the frame budget got involved. The trailing wedge is
a quad strip whose alpha falls off along its length, and upstream sets that with
`glMaterialfv` between columns; material is batch state here, so that is a draw
call per column, forty rings by forty-four columns, 1,760 of them for the sweep
alone. Turning on `GL_COLOR_MATERIAL` makes it per-vertex data instead and the
sweep is one call. It is the same property being set either way, since
colour-material tracks `GL_AMBIENT_AND_DIFFUSE` by default.

Its two team-name knobs are the one thing not in the panel: they are `<string>`
in the XML and the panel has no text field, so they are read from the query
string and nothing else. There are six string options in all 314 savers.

`unicrud` is the one that is genuinely blocked, and on two things at once. It
picks a codepoint at random from 0 to 0x2F800 and draws it four inches high, so
it wants a font covering essentially the whole Basic Multilingual Plane plus the
CJK compatibility ideographs; `runtime::font` is one compiled-in bitmap font of
Latin glyphs, and the alternative is megabytes of CJK outlines. It also captions
each character with its Unicode name, which upstream obtains by shelling out to
perl, noting in a comment that the only alternative is to embed the 943 KB of
NamesList.txt. Neither half has a small answer.

`worldpieces` is deferred on a dependency too, and a bigger one. It cuts each
country's outline into a mesh with Shewchuk's Triangle, asking for a quality
constrained Delaunay triangulation with holes, a minimum angle, a maximum area
and a bounded number of Steiner points. `runtime::tess` is ear clipping and
`runtime::delaunay` is the plain unconstrained kind; neither is that, and
Triangle itself is 638 KB of C whose whole difficulty is getting its geometric
predicates right in floating point. It is not blocked on anything about the
Earth: `runtime::dymaxion` and the maps are both in.

`extrusion` is deferred on a dependency rather than on anything about its own
552 lines: it draws everything through GLE, the tubing and extrusion library,
which upstream does not bundle and links against. Porting it means writing
`gleExtrusion`, `gleTwistExtrusion`, `gleSuperExtrusion`, `gleHelicoid`,
`gleScrew` and `gleTaper` first, with their join styles, which is a library in
its own right.

`sphereeversion` is two savers under one slug, and both are in. Upstream picks
at random between an analytic eversion, a closed-form formula from Bednorz and
Bednorz (2019), and the corrugations of the 1994 film "Outside In". They share
everything but the surface: the same twelve options, the same turn between
eversions, the same colourings.

The two are built completely differently, which is the interesting part. The
analytic one evaluates a formula and its two partial derivatives at 257 by 257
points of the sphere. The corrugations one is built out of *jets*: a value
carried along with its own derivatives, so that every operation propagates them
and the surface normals fall out of evaluating the formula rather than having to
be differentiated by hand. It evaluates one lune of the sphere and draws it
sixteen times, eight lunes to a hemisphere, which is how the whole sphere is
made out of one belt.

Both halves have a fixed-function path beside their GLSL one, and unlike
`timetunnel`'s those fallbacks are the whole saver rather than a stub, so the
fallback is what is ported. The one thing it cannot do is the earth colouring,
which upstream wraps day, night and water textures around the sphere in a
fragment shader; upstream's own fixed-function path quietly draws the plain
two-sided red and green instead, and so does this. The option stays, because
upstream's does.

One thing the port had to solve rather than copy. Upstream draws the surface
once with culling off and two-sided lighting, setting `GL_FRONT` and `GL_BACK`
materials *per vertex*, which real GL allows inside a block and this recorder
cannot: a material is batch state, so changing it every vertex would put every
vertex in its own draw call. A vertex colour is not state. So the surface is
drawn twice, culling one side each time, with the front colours on the first
pass and the back colours on the second. A triangle only ever shows one of its
faces to the camera, so the two passes cover exactly the fragments the single
unculled pass would have. It costs twice the vertices, and only for the two
colourings that vary over the surface: 267k in 527 strips at the heaviest
setting, against 133k for the two-sided colouring, which needs one pass.

The corrugations half needed one more thing. Upstream opens a block per strip,
which is sixty-four draw calls a lune and two thousand a frame here, since a
triangle strip cannot merge with the strip beside it. So the strips of a lune
are joined into one with the usual pair of repeated vertices between them: they
make two triangles of no area, which raster to nothing, and every strip is an
even number of vertices long so the winding of what follows a join is
unchanged. Two thousand draw calls became thirty-two, for four thousand more
vertices out of half a million.

`covid19` is the deferral that was exactly backwards, and it is worth writing
down why, because the reasoning that deferred it looks sound. A hundred virions
of a hundred spikes each is a great deal of geometry, so it was put aside on
volume. But upstream builds every model *twice*, coarse and fine, and switches
to the coarse ones as soon as there are more than forty on screen. So its
default of sixty is its cheap configuration and a dozen large ones is its
expensive one: measured, a coarse model is 4762 vertices and a fine one 152366,
so sixty coarse ones come to 286k a frame where forty fine ones would have come
to 6.1 million. The lesson is to find the setting the saver actually runs at
before counting its vertices at the setting you imagined.

What did have to be solved is that a virion is 239 separate triangle strips
coarse and 936 fine, so sixty of them drawn the way upstream draws them would be
fourteen thousand draw calls. Each of the twenty models is baked once into a
single joined strip, and a virion is then one call.

`co____9` is the same saver under a name that does not date it, which is what
upstream ships to the App Store. It has a configuration file of its own, so it
has a slug of its own here too, pointing at the same code.

`cubocteversion` is the same eversion done in straight lines, and the contrast
with `sphereeversion` is the point of having both. There is no formula and no
jet: it keeps twelve vertices, thirty edges and twenty flat triangles the whole
way, and everts by moving the vertices along straight lines from one polyhedron
to the next. Richard Denner and Francois Apery each worked out a sequence of
them, forty-five and seven, and the eversion *is* those tables, eased so the
corners do not show.

What is not tabulated is where the surface passes through itself, and that is
the part worth knowing about. It is found rather than modelled: every pair of
non-adjacent triangles is intersected against every other, every frame, by
Devillers and Guigue's predicate, and the segments that come back are drawn as
orange tubes. Both sets of tubes are balls and cylinders, up to twelve hundred
triangle strips of them, and they are joined the same way the corrugations
eversion's lunes are, so the whole plumbing is two draw calls.

Its transparency knob is not offered here. It chose between two depth-peeling
schemes, which are a way of drawing correct transparency in a fragment shader;
the fixed-function path has none of that and neither has this, so the knob
would have had nothing to select. Its earth colouring is in the same position
as `sphereeversion`'s, and upstream's fixed-function path likewise declines to
pick it at random.

`timetunnel` was deferred for a different reason, and it is the case where
following upstream's own fallback would have been the wrong answer. The two
functions that draw its signs and its tunnels are wrapped in `#ifndef
HAVE_JWZGLES`, so upstream's OpenGL ES build draws neither and a phone shows a
wall tunnel and nothing else. Every other saver that has a fixed-function
fallback has one worth porting; this one's fallback is the saver with most of
itself missing.

The three things it wanted turned out to be one small runtime addition each,
which is the general lesson: measure what a missing feature costs before
calling it a wall. `glBlendColor` and `glBlendEquation` are both plain WebGL 2
calls, so `Blend` grew `ConstantFade`, `ConstantAdd` and `ConstantSubtract` and
the host sets the constant and the equation alongside the factors. The texture
matrix is the one that stayed out of the runtime, because only this saver has
ever wanted one: it carries a 2x3 affine by hand and applies it as each
coordinate is written, the way `dymaxionmap` does. What that costs is that its
tunnels cannot live in a display list, since a list would replay the
coordinates already transformed; they are rebuilt each frame instead, and a
tunnel is thirty quads.

A saver whose crowd is a *setting* can keep its code and lower the setting
instead. `winduprobot` draws twenty-five robots of sixty-three thousand
vertices each, which is 1.6 million a frame; its default here is five, which
comes to the same 334k `beats` draws, and its slider still goes to a hundred.
Say so in the saver's own comment when you do this, with the measurement.

`hopffibration` is the one to look at before deferring a saver on its geometry,
because measuring the wrong quantity nearly lost it. Its fibers are tubes swept
along curves subdivided until they are smooth, and its heaviest animation draws
two hundred and sixteen of them: 767k vertices a frame at the coarsest of the
three detail levels its knob offers, and 1.6 million at upstream's default. On
the heaviest animation at the default setting it looks impossible. But the
heaviest animation is four of a hundred and eighty-eight, and at coarse the
median is 256k a frame and the upper quartile 343k, which is exactly what
`beats` draws. So it ships, with coarse as the default here and the knob still
offering the other two, the same accommodation `winduprobot` gets. The geometry
really is rebuilt every frame, because it moves every frame; upstream streams
it too, with `GL_STREAM_DRAW`. Measure the distribution, not the maximum.

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

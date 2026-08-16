# The download

2026-07-31. What a device actually fetches on a first visit, what it could fetch,
and what each reduction costs. Measured against the deployed site and against
real builds — no estimates.

## First, a correction

`docs/assembly-load.md` said 4.4 MB per device. That is the size **on disk**. The
server compresses, so what a phone actually downloads today is **~1.7 MB**:

| asset | on disk | over the wire | why |
|-|-|-|-|
| wasm | 3,990 KB | **1,521 KB** | gzip |
| style.css | 127 KB | 23 KB | gzip |
| JS glue | 70 KB | 17 KB | gzip |
| material-icons.woff2 | 125 KB | **125 KB** | woff2 is already compressed; gzip does nothing |
| Atkinson 400 + 700 | 34 KB | 34 KB | same |
| Atkinson italics | 36 KB | 0 KB | fetched only if italic text is rendered |
| other css | 12 KB | ~4 KB | gzip |
| **total** | **4.4 MB** | **~1.72 MB** | |

For 500 people that is ~860 MB through the venue uplink, not 2.2 GB. Still the
largest single thing the assembly will pull, and still worth halving.

## What is in the wasm

Nothing wasteful at the section level. The shipped binary carries **no debug
sections and no name section** — `scripts/split-symbols.nu` moves the DWARF into a
29 MB sidecar that only the backend ever fetches, to turn crash reports into
source lines. Of the 3,990 KB: 3,544 KB is code (6,685 functions) and 500 KB is
data. It is all program.

The build was already tuned: `[profile.wasm-release]` has `lto = "fat"`,
`codegen-units = 1`, `panic = "abort"`, and wasm-opt runs at `-Oz`. Roster parsing
(calamine/zip) was moved to the backend earlier for the same reason.

## The three levers, measured

### 1. `opt-level = "s"` → `"z"` — 214 KB, free

Built both and measured the same commit:

| | raw | gzip | brotli |
|-|-|-|-|
| `s` (what shipped) | 3,990 KB | 1,433 KB | 992 KB |
| `z` | **3,263 KB** | **1,219 KB** | **879 KB** |

18% smaller before compression. `z` tells LLVM to stop inlining for speed; in an
app that spends its life waiting on the network, that trade is one-sided. The
`z` build was loaded in a browser and renders correctly.

**Applied.**

### 2. Brotli instead of gzip — ~340 KB, not ours to switch on

statichost serves **gzip only**. Asking for `Accept-Encoding: br` returns the
uncompressed file. On the same wasm, brotli -11 against gzip -9 is
**1,219 KB → 879 KB**.

That is the biggest remaining number, and no code change can match it. Two ways:

- **Ask statichost to enable brotli.** Cleanest, costs nothing, benefits every
  asset. Worth an email before the assembly.
- **Ship pre-compressed bytes** with `Content-Encoding: br` set on that path in
  `_headers` (which statichost does honour — the `immutable` cache rule proves
  it). The catch is that it is unconditional: a client that does not accept
  brotli gets bytes it cannot read. Every browser that can run wasm has supported
  brotli for years, so the practical risk is low, but it is a real one, and it
  should be probed with a throwaway file on a deploy before the wasm depends on
  it.

### 3. Subset the icon font — 89 KB, verified safe

The font carries **2,233 icons**; the app can render **105** of them. Subsetting
to those (`pyftsubset --layout-features=liga`) gives **125 KB → 36 KB**.

Verified rather than assumed: both fonts were loaded in a browser and every one
of the 105 names measured with canvas `measureText`. **Zero width mismatches
against the full font, and zero ligatures that failed to form** — so every icon
still renders exactly as before.

Not yet applied, because doing it properly means generating the subset at build
time from the full font (so a newly-used icon cannot silently render as the word
`add_reaction`), which needs `fonttools` in the devshell and the Nix package. The
guard rail worth having with it: the icon list is derived by intersecting every
string literal in `src/` with the font's own ligature names, so a test can hold
that same invariant without parsing a font.

## Also worth doing, no bytes involved

**Preload the wasm.** The served `index.html` preloads the 17 KB JS glue but not
the 1.5 MB wasm, so the browser only learns about the big download after the glue
has arrived and started — an extra round trip on exactly the connection where
round trips hurt. A `<link rel="preload">` injected at build time (the filename is
content-hashed, so it has to be injected, not templated) starts both at once.
Needs checking in a browser first: a preload whose `as`/`crossorigin` do not match
the real request downloads the file **twice**, which would be worse than the
problem.

## Rejected, with reasons

- **Dropping the DWARF line tables** (`debug = "line-tables-only"`) saves ~2% and
  costs every crash report its source line. Not worth it.
- **The italic faces** are already only fetched when italic text renders.
- **The CSS** is 23 KB gzipped. There is nothing there.
- **The 29 MB symbol sidecar** is never fetched by a reader — it is already off
  the critical path.

## Where it lands

| | today | with all three |
|-|-|-|
| per device | ~1.72 MB | **~0.95 MB** |
| 500 devices | ~860 MB | **~470 MB** |

## The next frontier, unmeasured

Dependency-level: `reqwest` (with multipart), `cynic`, `serde_json`,
`material-colors`, `dioxus-primitives`. I could not attribute size to them — the
shipped wasm is stripped and the sidecar has DWARF but no name section, so twiggy
reports `code[0]`, `code[1]`… To do it properly, build once retaining the name
section and run `twiggy top` against that. Only worth it if the numbers above
turn out not to be enough; each of them is bigger than most dependency wins, and
none of them risks changing behaviour.

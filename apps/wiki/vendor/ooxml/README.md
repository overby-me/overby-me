# Vendored Office Open XML parsers

From [office-open-xml-viewer](https://github.com/yukiyokotani/office-open-xml-viewer)
by Yuki Yokotani, MIT licensed. The licence is in `LICENSE` beside this file and
must stay there.

Pinned at commit `d8c4b961adce0b90e43cbe7723a39f3f123b9e74`.

Four crates: `ooxml-common`, `docx-parser`, `xlsx-parser`, `pptx-parser`. They
turn a `.docx` / `.xlsx` / `.pptx` into a JSON document model, which
`components::docx`, `components::xlsx` and `components::pptx` render.

## Why vendored rather than a git dependency

It started as a pinned git dependency, which is what this repo does elsewhere
(see `dioxus-primitives`). That works for one of these crates and fails for two:

```text
warning: Linking globals named '__wbindgen_describe_extract_image':
         symbol multiply defined!
error: failed to load bitcode of module "pptx_parser-….rcgu.o"
```

All three export a `#[wasm_bindgen] pub fn extract_image`, and wasm-bindgen
generates a descriptor symbol named after the function. Two crates in one wasm
binary means two definitions of the same global, and the link fails. There is no
feature flag to turn the JS layer off — the attributes are unconditional — so
the only way to have all three is to change the sources.

## What was changed

Two mechanical edits, applied to every file:

1. **Every `#[wasm_bindgen…]` attribute line removed** (20 of them). They exist
   to generate a JavaScript binding; this app calls the functions as ordinary
   Rust and never needed one. Removing them removes the colliding symbols.
2. **`#[cfg(not(target_arch = "wasm32"))]` removed** from the `*_native`
   entry points (2 of them). Upstream gates those off for wasm because they are
   there for its MCP server; they are precisely the plain-Rust API this app
   wants, and there is no reason they cannot exist on wasm.

`Cargo.toml` for each crate additionally had `{ workspace = true }` dependency
specs replaced with concrete versions (the crates were lifted out of their own
workspace) and `crate-type` narrowed from `["cdylib", "rlib"]` to `["rlib"]`,
since nothing here builds them as a standalone wasm module.

**No parsing logic was touched.** If these need updating, re-vendor from
upstream and re-apply the two edits above rather than hand-patching.

## A note on file size

`docx-parser/src/parser.rs` is a shade over 1 MiB, which is above jj's default
`snapshot.max-new-file-size`. It was silently refused on the first commit — the
tree still built locally, because the file was on disk, but the COMMIT did not
contain it and a clean checkout would not have compiled.

The repo config now allows 4 MiB (`jj config set --repo
snapshot.max-new-file-size 4MiB`). That setting lives in `.jj/repo/config.toml`,
which is not itself tracked, so anyone re-vendoring from scratch has to set it
again. Check `jj file list vendor/ooxml` against `find vendor/ooxml -name '*.rs'`
if a vendored build ever fails on a fresh clone.

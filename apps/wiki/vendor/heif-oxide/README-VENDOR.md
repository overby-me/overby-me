# Vendored heif-oxide

`heif-oxide` 0.1.0, unpacked from crates.io (MIT OR Apache-2.0, dan335;
`README.md`, `testdata/` and the `src/*_tests.rs` files are upstream's and
stay). Wired in as a `path` dependency from `../../Cargo.toml`.

`examples/decode.rs` was dropped, along with its `[[example]]` section: it is a
native CLI that reads a file off disk, which this target has no use for.

`Cargo.toml` also has its keys reordered against upstream's, by the tree's
`tombi-format` pre-commit hook. Nothing was added or removed by it, but expect
a diff against a fresh `cargo package` output on re-vendoring.

## The one change

Upstream decodes grid tiles, and converts YUV to RGB, on
`std::thread::scope`/`spawn`:

```rust
let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
let chunk_size = tiles.len().div_ceil(threads).max(1);
std::thread::scope(|s| {
    for (tile_chunk, result_chunk) in ... {
        s.spawn(move || { ... });
```

`wasm32-unknown-unknown` in a browser has no threads, so `spawn` panics with
`failed to spawn thread: Error { kind: Unsupported }`, and under
`panic = "abort"` that kills the page rather than just the decode. It reached
us as a white screen on any candidate whose photo came off an iPhone.

The work was already down to one worker there: `available_parallelism` returns
`Err` on wasm, so `unwrap_or(1)` makes a single chunk. Upstream still spawned a
thread to run that one chunk, and that alone is what died.

`src/par.rs` (added) is `std::thread::scope` on every other target, and on wasm
a `scope`/`spawn` pair that runs each closure inline. The three call sites
(one in `src/lib.rs`, two in `src/color.rs`) now say `crate::par::scope`. Native
builds are unchanged, and on wasm the same chunks are computed in the same
order, on the calling thread.

Not reported upstream yet: the crate has published only 0.1.0 and the repo
(<https://github.com/dan335/heif-oxide>) shows no wasm target support, so this
is arguably a feature request rather than a bug.

## On upgrading heif-oxide

Re-vendor from the new release, drop `examples/`, and re-apply the change (or
drop the vendoring entirely, and depend on the registry crate again, once
upstream builds for wasm).

# heif-oxide

Pure-Rust HEIF/HEIC still-image decoder. No C dependencies, no `unsafe`,
MIT OR Apache-2.0.

Parses the HEIF container (ISO/IEC 23008-12) and decodes the HEVC payload
with [`rust_h265`](https://crates.io/crates/rust_h265) — a pure-Rust HEVC
decoder. To our knowledge this is the first permissively-licensed way to
open iPhone photos in Rust without linking libheif/libde265.

```rust
let image = heif_oxide::decode_file("photo.heic")?;
println!("{}x{}, {} channels, {}-bit",
    image.width, image.height, image.channels(), image.bit_depth());
let rgba: Vec<u8> = image.to_rgba8();          // or…
let f32s: Vec<f32> = image.to_f32_interleaved(); // 0.0..=1.0, native channels
```

## What works

| Feature | Status |
| --- | --- |
| Single-picture `hvc1` items | ✅ |
| Grid-tiled images (every iPhone photo) | ✅ tiles decoded in parallel |
| 8-bit and 10-bit HEVC (Main / Main 10) | ✅ 10-bit → 16-bit output |
| Orientation (`irot` / `imir` / `clap`) | ✅ applied in `ipma` order |
| BT.601 / BT.709 / BT.2020 matrices, full & limited range | ✅ |
| Display P3 → sRGB conversion (iPhone default) | ✅ linear-light 3×3 |
| Alpha auxiliary images | ⚠️ only when 4:2:0-coded (monochrome alpha needs a 4:0:0 decoder) |
| `idat`-stored payloads, multi-extent items | ✅ |
| Malformed input | errors, never panics (every parse is bounds-checked) |

Output is always display-ready sRGB.

## What doesn't (yet)

- Encoding — there is no pure-Rust HEVC encoder to build on.
- AVIF (`av01`) and JPEG-in-HEIF payloads — rejected with a clear error
  naming the codec.
- Image sequences (track/`moov`-based files), identity-derived (`iden`)
  items, overlays (`iovl`), multi-layer (`lhv1`) and 4:4:4 streams,
  protected items, external data references.
- ICC profiles are reported (`ColorInfo::icc_present`) but not applied;
  PQ/HLG HDR transfers are not tone-mapped.

## Status

On the [Nokia HEIF conformance suite](https://github.com/nokiatech/heif_conformance),
44 of 63 files decode; the rest are rejected cleanly for the unsupported
features listed above. Tested against real iPhone photos.

Decoding is slower than libheif (the HEVC decoder is scalar pure Rust,
roughly 5× slower than FFmpeg's) — a 12-megapixel iPhone photo takes on the
order of a second, with grid tiles spread across cores.

## Testing

`cargo test` exercises the container parser against hand-built boxes,
the colour pipeline against hand-computed reference values, and the full
decode path against tiny losslessly-encoded x265 fixtures (committed under
`testdata/`, generated from flat frames with exactly known plane values —
see `src/lib_tests.rs`).

## License

MIT OR Apache-2.0, at your option.

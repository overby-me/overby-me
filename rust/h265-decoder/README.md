# h265-decoder

A pure Rust H.265/HEVC decoder library.

Parses H.265 Annex B bytestreams and decodes frames to YUV 4:2:0 planar (or converted to RGB/RGBA/NV12), suitable for downstream consumers such as AV1 encoders (e.g. rav1e) or display pipelines.

## Project structure

```text
h265-decoder/
├── Cargo.toml                          # Workspace manifest
├── crates/
│   └── h265-decode/
│       ├── Cargo.toml                  # Library crate
│       └── src/
│           ├── lib.rs                  # Public API exports
│           ├── decode.rs               # Top-level Decoder struct and pipeline
│           ├── bitstream.rs            # Bit-level reader, exp-Golomb, emulation prevention
│           ├── nal.rs                  # HEVC NAL unit types, VPS/SPS/PPS/slice header parsing
│           ├── dpb.rs                  # Decoded Picture Buffer (HEVC RPS-based management)
│           ├── frame.rs                # DecodedFrame, FramePlane, PictureType
│           ├── pixel.rs                # PixelFormat, ColourMatrix, YUV↔RGB conversion
│           └── error.rs                # Error and warning types
├── default.nix                         # Nix package and devShell
├── justfile                            # Development commands
└── README.md
```

## Quick start

```rust
use h265_decode::{Decoder, DecoderConfig, PixelFormat};

let config = DecoderConfig::new().pixel_format(PixelFormat::Yuv420p);
let mut decoder = Decoder::new(config);

// Feed raw H.265 Annex B bytestream data
let h265_data: &[u8] = &[/* ... */];
let frames = decoder.decode(h265_data).expect("decode failed");

for frame in &frames {
    println!(
        "decoded {}x{} frame, pts={:?}, {} bytes",
        frame.width(),
        frame.height(),
        frame.pts(),
        frame.data().len(),
    );
}

// Flush remaining frames at end of stream
let trailing = decoder.flush().expect("flush failed");
```

## Architecture

```text
H.265 Annex B bytestream
        │
        ▼
┌───────────────────────────┐
│  Annex B start-code       │  Split byte stream on 00 00 01 / 00 00 00 01
│  scanner                  │  start codes into individual NAL units
└───────┬───────────────────┘
        │  Vec<NAL unit bytes>
        ▼
┌───────────────────────────┐
│  NAL unit parser          │  2-byte HEVC NAL header (type, layer, temporal ID)
│  + emulation prevention   │  Remove 0x00 0x00 0x03 escape bytes → RBSP
│  removal                  │
└───────┬───────────────────┘
        │  NAL type + RBSP data
        ▼
┌───────────────────────────┐
│  Parameter set parsers    │  VPS (§7.3.2.1): layer/sub-layer config
│  VPS / SPS / PPS          │  SPS (§7.3.2.2): resolution, chroma, bit depth,
│                           │      coding block sizes, reference picture sets
│                           │  PPS (§7.3.2.3): QP, tiles, deblocking, etc.
└───────┬───────────────────┘
        │
        ▼
┌───────────────────────────┐
│  Slice header parser      │  Slice type (I/P/B), POC LSB, RPS selection,
│  (§7.3.6.1)               │  segment address, dependent slice handling
└───────┬───────────────────┘
        │
        ▼
┌───────────────────────────┐
│  POC derivation           │  Picture Order Count from LSB + MSB accumulator
│  (§8.3.1)                 │  (same algorithm as H.264 POC type 0)
└───────┬───────────────────┘
        │
        ▼
┌───────────────────────────┐
│  Picture reconstruction   │  Scaffold: grey I-frames, reference-copy P/B
│  (CTU decode — WIP)       │  Future: CABAC, intra/inter pred, DST/DCT,
│                           │  deblocking, SAO
└───────┬───────────────────┘
        │  DecodedFrame (Y, U, V planes)
        ▼
┌───────────────────────────┐
│  Decoded Picture Buffer   │  HEVC RPS-based reference management
│  (DPB)                    │  Reorder bumping (max_num_reorder_pics)
│                           │  Display-order output (ascending POC)
└───────┬───────────────────┘
        │  Frames in display order
        ▼
┌───────────────────────────┐
│  Pixel format conversion  │  YUV 4:2:0 → RGB24 / RGBA32 / NV12
│  (optional)               │  BT.601 / BT.709 / BT.2020 matrices
└───────────────────────────┘
```

## HEVC features implemented

### NAL layer

- [x] 2-byte HEVC NAL unit header parsing (type, layer ID, temporal ID)
- [x] All VCL and non-VCL NAL unit type classification
- [x] IRAP detection (IDR, CRA, BLA)
- [x] Annex B start-code scanning (3-byte and 4-byte)
- [x] Emulation prevention byte removal (0x00 0x00 0x03)

### Parameter sets

- [x] Video Parameter Set (VPS) – profile/tier/level, sub-layer ordering
- [x] Sequence Parameter Set (SPS) – resolution, chroma format, bit depth, coding block sizes, reference picture sets, long-term refs, PCM, scaling list (skipped)
- [x] Picture Parameter Set (PPS) – QP, tiles, deblocking, weighted prediction, entropy coding sync
- [x] Profile/Tier/Level parsing (general + sub-layer skip)
- [x] Short-term Reference Picture Set parsing (explicit + inter-prediction)
- [x] Conformance window crop offset computation

### Slice layer

- [x] Slice segment header: first_slice flag, type (I/P/B), POC LSB, RPS selection
- [x] Dependent slice segment detection
- [x] Slice-local short-term RPS parsing

### Picture management

- [x] POC derivation (MSB/LSB accumulator with wraparound)
- [x] Decoded Picture Buffer with configurable size
- [x] RPS-based reference marking (mark unused, short-term, long-term)
- [x] Reorder bumping (max_num_reorder_pics constraint)
- [x] Display-order output (ascending POC)
- [x] IDR DPB flush
- [x] Reference list 0 and list 1 construction

### Pixel output

- [x] YUV 4:2:0 planar (native)
- [x] RGB24 packed conversion
- [x] RGBA32 packed conversion
- [x] NV12 semi-planar conversion
- [x] BT.601, BT.709, BT.2020 colour matrices

### Reconstruction (scaffold)

- [x] Grey placeholder for I-frames
- [x] Reference copy for P/B-frames
- [ ] CABAC entropy decoding
- [ ] Intra prediction (planar, DC, angular)
- [ ] Inter prediction (motion compensation)
- [ ] Inverse transform (DST 4×4, DCT 8×8/16×16/32×32)
- [ ] Dequantization
- [ ] Deblocking filter
- [ ] Sample Adaptive Offset (SAO)

## Pixel format output

| Format | Description | Use case |
|--------|-------------|----------|
| `Yuv420p` | YUV 4:2:0 planar (I420) | Video encoders (rav1e, x265) |
| `Rgb24` | Packed 24-bit RGB | Display, image processing |
| `Rgba32` | Packed 32-bit RGBA | Display with alpha |
| `Nv12` | Semi-planar Y + interleaved UV | Hardware APIs (VA-API, DXVA) |

## Development

```bash
# Build
cargo build

# Run all tests (138 unit tests)
cargo test

# Run clippy
cargo clippy -- -D warnings

# Format code
cargo fmt
```

### Nix

```bash
# Enter the devShell
direnv allow  # or: nix develop .#h265-decoder

# Build the package
nix build .#h265-decoder
```

## Comparison with h264-decoder

| Feature | h264-decoder | h265-decoder |
|---------|----------------|----------------|
| NAL header | 1 byte | 2 bytes |
| Parameter sets | SPS + PPS | VPS + SPS + PPS |
| Coding unit | Macroblock (16×16) | CTU (up to 64×64) |
| Reference management | Sliding window / MMCO | Reference Picture Sets |
| POC derivation | Type 0, 1, 2 | Single type (LSB/MSB) |
| Colour matrix default | BT.601 | BT.709 |
| Colour matrices | BT.601, BT.709 | BT.601, BT.709, BT.2020 |
| NAL parser | h264-reader crate | Built-in (pure Rust) |
| Reconstruction | Scaffold | Scaffold |

## License

MIT
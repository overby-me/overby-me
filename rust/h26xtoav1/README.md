# h26xtoav1

A CLI tool for transcoding H.264 (AVC) and H.265 (HEVC) video to AV1, built entirely in Rust.

Uses [`h264-decode`](../h264-decoder) and [`h265-decode`](../h265-decoder) for decoding and [rav1e](https://github.com/xiph/rav1e) for AV1 encoding. Output is written as an [IVF](https://wiki.multimedia.cx/index.php/IVF) container.

## Project structure

```text
h26xtoav1/
├── Cargo.toml          # Package manifest
├── src/
│   ├── main.rs         # CLI entry point, decode/encode pipeline
│   └── ivf.rs          # IVF container writer
├── default.nix         # Nix package and devShell
├── justfile            # Development commands
└── README.md
```

## Usage

```bash
# Build
cargo build --release

# Basic transcode – codec is auto-detected from extension
h26xtoav1 input.264 -o output.ivf     # H.264 input
h26xtoav1 input.265 -o output.ivf     # H.265 input
h26xtoav1 input.hevc -o output.ivf    # H.265 input

# Force input codec (overrides auto-detection)
h26xtoav1 input.bin -o output.ivf --codec h264
h26xtoav1 input.bin -o output.ivf --codec h265

# Specify speed preset and quantizer
h26xtoav1 input.264 -o output.ivf -s 4 -q 80

# Fast preview encode
h26xtoav1 input.265 -o output.ivf -s 10 -q 128

# High quality encode
h26xtoav1 input.264 -o output.ivf -s 3 -q 60

# Low-latency mode (no frame reordering)
h26xtoav1 input.265 -o output.ivf --low-latency

# Limit to first 100 frames with verbose logging
h26xtoav1 input.264 -o output.ivf --max-frames 100 -v

# Write to stdout (e.g. pipe to ffplay)
h26xtoav1 input.265 -o - | ffplay -i -
```

## CLI options

```text
h26xtoav1 [OPTIONS] <INPUT> -o <OUTPUT>

Arguments:
  <INPUT>                  Path to the input H.264/H.265 Annex B file
                           (.264/.h264/.265/.h265/.hevc)

Options:
  -o, --output <OUTPUT>    Output AV1 file path (.ivf), or "-" for stdout
  -s, --speed <SPEED>      rav1e speed preset (0=slowest/best .. 10=fastest) [default: 6]
  -q, --quantizer <QP>     Quantizer (0=lossless .. 255=worst) [default: 100]
      --threads <N>        Number of encoding threads (0 = auto) [default: 0]
      --fps <FPS>          Frame rate to assume for the output [default: 30]
      --keyint <N>         Maximum keyframe interval [default: 240]
      --low-latency        Enable low-latency mode (single-pass, no reordering)
      --max-frames <N>     Maximum number of frames to transcode (0 = unlimited) [default: 0]
      --codec <CODEC>      Force input codec: auto, h264, or h265 [default: auto]
  -v, --verbose            Enable verbose logging
  -h, --help               Print help
  -V, --version            Print version
```

## Codec auto-detection

When `--codec auto` (the default), the input codec is determined by:

1. **File extension** – `.264`, `.h264`, `.avc` → H.264; `.265`, `.h265`, `.hevc` → H.265.
2. **Content inspection** – if the extension is ambiguous, the first NAL unit header is examined to distinguish between H.264 (1-byte header) and H.265 (2-byte header) parameter sets.

Use `--codec h264` or `--codec h265` to override when auto-detection cannot determine the format.

## Pipeline

```text
H.264/H.265 Annex B file
        │
        ▼
┌───────────────────────┐
│  h264-decode /        │  Pure Rust decoders
│  h265-decode          │  Parse NAL units, decode frames
│  (YUV 4:2:0 out)      │
└───────┬───────────────┘
        │  DecodedFrame (Y, U, V planes)
        ▼
┌───────────────────────┐
│  rav1e                │  AV1 encoder
│  (AV1 packets)        │  Encodes YUV frames to AV1
└───────┬───────────────┘
        │  Encoded packets
        ▼
┌───────────────────────┐
│  IVF writer           │  Minimal container format
│  (.ivf output)        │  32-byte header + per-frame headers
└───────────────────────┘
```

## Playing the output

The IVF container is widely supported:

```bash
# Play with ffplay
ffplay output.ivf

# Play with mpv
mpv output.ivf

# Remux to MP4 with ffmpeg
ffmpeg -i output.ivf -c copy output.mp4

# Remux to WebM
ffmpeg -i output.ivf -c copy output.webm
```

## Speed vs quality tradeoffs

| Preset | Speed | Quality | Use case |
|--------|-------|---------|----------|
| 0 | Slowest | Best | Archival, final encode |
| 3 | Slow | Very good | High-quality distribution |
| 6 | Medium | Good | General purpose (default) |
| 8 | Fast | Fair | Quick preview |
| 10 | Fastest | Lower | Real-time / testing |

Lower quantizer values produce higher quality (and larger files). A quantizer of 0 is lossless.

## Development

```bash
# Run all checks
just all

# Build
just build

# Run tests
just test

# Run clippy
just clippy

# Format code
just fmt

# Quick transcode (H.264)
just transcode input.264

# Quick transcode (H.265)
just transcode input.265

# High-quality transcode
just transcode-hq input.264 output.ivf
```

### Nix

```bash
# Enter the devShell
direnv allow  # or: nix develop .#h26xtoav1

# Build the package
nix build .#h26xtoav1
```

## License

MIT
//! CLI tool for transcoding H.264/H.265 Annex B bytestream files to AV1 (IVF container).
//!
//! # Usage
//!
//! ```text
//! h26xtoav1 [OPTIONS] <INPUT> -o <OUTPUT>
//!
//! Arguments:
//!   <INPUT>  Path to the input H.264/H.265 Annex B file (.264/.h264/.265/.h265/.hevc)
//!
//! Options:
//!   -o, --output <OUTPUT>      Output AV1 file path (.ivf)
//!   -s, --speed <SPEED>        rav1e speed preset (0=slowest/best .. 10=fastest) [default: 6]
//!   -q, --quantizer <QP>       Quantizer (0=lossless .. 255=worst) [default: 100]
//!       --threads <N>          Number of encoding threads (0 = auto) [default: 0]
//!       --fps <FPS>            Frame rate to assume for the output [default: 30]
//!       --max-frames <N>       Maximum number of frames to transcode (0 = unlimited) [default: 0]
//!       --keyint <N>           Maximum keyframe interval [default: 240]
//!       --low-latency          Enable low-latency mode (single-pass, no reordering)
//!       --codec <CODEC>        Force input codec: h264, h265, or auto [default: auto]
//!   -v, --verbose              Enable verbose logging
//!   -h, --help                 Print help
//!   -V, --version              Print version
//! ```

mod ivf;

use std::fs;
use std::io::{self, BufWriter, Cursor, Seek, Write};
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use rav1e::prelude::*;

use crate::ivf::IvfWriter;

// ---------------------------------------------------------------------------
// Input codec detection
// ---------------------------------------------------------------------------

/// Supported input video codecs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum InputCodec {
    /// Automatically detect from file extension and content.
    Auto,
    /// H.264 / AVC.
    H264,
    /// H.265 / HEVC.
    H265,
}

impl std::fmt::Display for InputCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InputCodec::Auto => write!(f, "auto"),
            InputCodec::H264 => write!(f, "h264"),
            InputCodec::H265 => write!(f, "h265"),
        }
    }
}

/// Detect the input codec from the file extension.
fn detect_codec_from_extension(path: &std::path::Path) -> Option<InputCodec> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    match ext.as_str() {
        "264" | "h264" | "avc" => Some(InputCodec::H264),
        "265" | "h265" | "hevc" => Some(InputCodec::H265),
        _ => None,
    }
}

/// Detect the input codec by inspecting the first few NAL units.
///
/// H.264 NAL headers are 1 byte: `forbidden_zero_bit(1) | nal_ref_idc(2) | nal_unit_type(5)`.
/// H.265 NAL headers are 2 bytes: `forbidden_zero_bit(1) | nal_unit_type(6) | nuh_layer_id(6) | nuh_temporal_id_plus1(3)`.
///
/// We look at the first NAL unit after the start code and check if it looks
/// like an H.264 SPS (type 7) or H.265 VPS (type 32).
fn detect_codec_from_content(data: &[u8]) -> Option<InputCodec> {
    // Find the first start code.
    let mut i = 0;
    while i + 3 < data.len() {
        if data[i] == 0x00 && data[i + 1] == 0x00 {
            if data[i + 2] == 0x01 {
                let nal_start = i + 3;
                return classify_first_nal(&data[nal_start..]);
            } else if i + 4 < data.len() && data[i + 2] == 0x00 && data[i + 3] == 0x01 {
                let nal_start = i + 4;
                return classify_first_nal(&data[nal_start..]);
            }
        }
        i += 1;
    }
    None
}

fn classify_first_nal(nal_data: &[u8]) -> Option<InputCodec> {
    if nal_data.is_empty() {
        return None;
    }

    let first_byte = nal_data[0];

    // H.264 check: first byte is the NAL header.
    // nal_unit_type = first_byte & 0x1F
    let h264_type = first_byte & 0x1F;
    // Common H.264 first NAL types: SPS=7, PPS=8, AUD=9, IDR=5
    if matches!(h264_type, 6..=9) {
        return Some(InputCodec::H264);
    }

    // H.265 check: 2-byte header.
    // nal_unit_type = (first_byte >> 1) & 0x3F
    let h265_type = (first_byte >> 1) & 0x3F;
    // Common H.265 first NAL types: VPS=32, SPS=33, PPS=34, AUD=35
    if matches!(h265_type, 32..=35) {
        return Some(InputCodec::H265);
    }

    // H.264 slice types: IDR=5, non-IDR=1
    if matches!(h264_type, 1 | 5) {
        return Some(InputCodec::H264);
    }

    // H.265 VCL types: TRAIL_N=0..CRA=21
    if h265_type <= 21 && nal_data.len() >= 2 {
        // Verify second byte looks like valid H.265 (temporal_id_plus1 should be >= 1)
        let tid_plus1 = nal_data[1] & 0x07;
        if tid_plus1 >= 1 {
            return Some(InputCodec::H265);
        }
    }

    None
}

/// Determine the input codec, trying extension first, then content detection.
fn resolve_codec(cli_codec: InputCodec, path: &std::path::Path, data: &[u8]) -> Result<InputCodec> {
    match cli_codec {
        InputCodec::H264 => Ok(InputCodec::H264),
        InputCodec::H265 => Ok(InputCodec::H265),
        InputCodec::Auto => {
            if let Some(codec) = detect_codec_from_extension(path) {
                return Ok(codec);
            }
            if let Some(codec) = detect_codec_from_content(data) {
                return Ok(codec);
            }
            bail!(
                "cannot detect input codec for {}; use --codec h264 or --codec h265",
                path.display()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Decoded frame abstraction
// ---------------------------------------------------------------------------

/// A codec-agnostic decoded frame that provides access to YUV planes.
///
/// This wraps either an h264-decode or h265-decode frame so that the
/// encode loop does not need to know which codec was used.
enum DecodedFrameRef<'a> {
    H264(&'a h264_decode::DecodedFrame),
    H265(&'a h265_decode::DecodedFrame),
}

#[allow(dead_code)]
impl DecodedFrameRef<'_> {
    fn width(&self) -> u32 {
        match self {
            DecodedFrameRef::H264(f) => f.width(),
            DecodedFrameRef::H265(f) => f.width(),
        }
    }

    fn height(&self) -> u32 {
        match self {
            DecodedFrameRef::H264(f) => f.height(),
            DecodedFrameRef::H265(f) => f.height(),
        }
    }

    fn y_data(&self) -> &[u8] {
        match self {
            DecodedFrameRef::H264(f) => f.y_plane().data(),
            DecodedFrameRef::H265(f) => f.y_plane().data(),
        }
    }

    fn y_stride(&self) -> usize {
        match self {
            DecodedFrameRef::H264(f) => f.y_plane().stride() as usize,
            DecodedFrameRef::H265(f) => f.y_plane().stride() as usize,
        }
    }

    fn u_data(&self) -> &[u8] {
        match self {
            DecodedFrameRef::H264(f) => f.u_plane().data(),
            DecodedFrameRef::H265(f) => f.u_plane().data(),
        }
    }

    fn u_stride(&self) -> usize {
        match self {
            DecodedFrameRef::H264(f) => f.u_plane().stride() as usize,
            DecodedFrameRef::H265(f) => f.u_plane().stride() as usize,
        }
    }

    fn v_data(&self) -> &[u8] {
        match self {
            DecodedFrameRef::H264(f) => f.v_plane().data(),
            DecodedFrameRef::H265(f) => f.v_plane().data(),
        }
    }

    fn v_stride(&self) -> usize {
        match self {
            DecodedFrameRef::H264(f) => f.v_plane().stride() as usize,
            DecodedFrameRef::H265(f) => f.v_plane().stride() as usize,
        }
    }
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

/// Transcode H.264 (AVC) or H.265 (HEVC) video to AV1.
///
/// Reads an H.264/H.265 Annex B bytestream, decodes it with a pure-Rust
/// decoder, re-encodes every frame to AV1 using rav1e, and writes the
/// output as an IVF container.
#[derive(Parser, Debug)]
#[command(name = "h26xtoav1", version, about)]
struct Cli {
    /// Path to the input H.264/H.265 Annex B file (.264/.h264/.265/.h265/.hevc).
    input: PathBuf,

    /// Output AV1 file path (.ivf).
    ///
    /// Use `-` to write to stdout.
    #[arg(short, long)]
    output: PathBuf,

    /// rav1e speed preset (0 = slowest/best quality, 10 = fastest).
    #[arg(short, long, default_value_t = 6)]
    speed: u8,

    /// Quantizer (0 = lossless, 255 = worst quality).
    #[arg(short, long, default_value_t = 100)]
    quantizer: usize,

    /// Number of encoding threads (0 = automatic).
    #[arg(long, default_value_t = 0)]
    threads: usize,

    /// Frame rate (frames per second) to assume for the output.
    #[arg(long, default_value_t = 30)]
    fps: u32,

    /// Maximum keyframe interval in frames.
    #[arg(long, default_value_t = 240)]
    keyint: u64,

    /// Enable low-latency mode (single-pass, no frame reordering).
    #[arg(long)]
    low_latency: bool,

    /// Maximum number of frames to transcode (0 = unlimited).
    #[arg(long, default_value_t = 0)]
    max_frames: usize,

    /// Force input codec instead of auto-detecting.
    #[arg(long, value_enum, default_value_t = InputCodec::Auto)]
    codec: InputCodec,

    /// Enable verbose (debug-level) logging.
    #[arg(short, long)]
    verbose: bool,
}

// ---------------------------------------------------------------------------
// Seekable output abstraction
// ---------------------------------------------------------------------------

/// Output target that implements both `Write` and `Seek`.
///
/// For file output we write directly; for stdout we buffer into memory
/// (since stdout is not seekable) and flush at the end.
enum Output {
    /// Buffered file writer (seekable).
    File(BufWriter<fs::File>),
    /// In-memory buffer, flushed to stdout on [`Output::finish`].
    Stdout(Cursor<Vec<u8>>),
}

impl Write for Output {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Output::File(w) => w.write(buf),
            Output::Stdout(c) => c.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Output::File(w) => w.flush(),
            Output::Stdout(c) => c.flush(),
        }
    }
}

impl Seek for Output {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        match self {
            Output::File(w) => w.seek(pos),
            Output::Stdout(c) => c.seek(pos),
        }
    }
}

impl Output {
    /// Create an [`Output`] for the given path.
    ///
    /// If the path is `"-"`, output will be buffered in memory and written
    /// to stdout when [`Output::finish`] is called.
    fn open(path: &PathBuf) -> Result<Self> {
        if path.to_str() == Some("-") {
            Ok(Output::Stdout(Cursor::new(Vec::new())))
        } else {
            let file = fs::File::create(path)
                .with_context(|| format!("cannot create output file {}", path.display()))?;
            Ok(Output::File(BufWriter::new(file)))
        }
    }

    /// Flush / finalise the output.
    ///
    /// For file output this simply flushes the buffered writer.  For stdout
    /// mode, the entire in-memory buffer is written to stdout.
    fn finish(self) -> Result<()> {
        match self {
            Output::File(mut w) => {
                w.flush().context("failed to flush output file")?;
            }
            Output::Stdout(cursor) => {
                let data = cursor.into_inner();
                let mut out = io::stdout().lock();
                out.write_all(&data).context("failed to write to stdout")?;
                out.flush().context("failed to flush stdout")?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Decode helpers
// ---------------------------------------------------------------------------

/// Decoded frames from either codec.
enum DecodedFrames {
    H264(Vec<h264_decode::DecodedFrame>),
    H265(Vec<h265_decode::DecodedFrame>),
}

impl DecodedFrames {
    fn len(&self) -> usize {
        match self {
            DecodedFrames::H264(v) => v.len(),
            DecodedFrames::H265(v) => v.len(),
        }
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn width(&self) -> u32 {
        match self {
            DecodedFrames::H264(v) => v.first().map_or(0, |f| f.width()),
            DecodedFrames::H265(v) => v.first().map_or(0, |f| f.width()),
        }
    }

    fn height(&self) -> u32 {
        match self {
            DecodedFrames::H264(v) => v.first().map_or(0, |f| f.height()),
            DecodedFrames::H265(v) => v.first().map_or(0, |f| f.height()),
        }
    }
}

fn decode_h264(input_data: &[u8], verbose: bool) -> Result<(DecodedFrames, Vec<String>)> {
    let decoder_config = h264_decode::DecoderConfig::new()
        .pixel_format(h264_decode::PixelFormat::Yuv420p)
        .collect_warnings(verbose);

    let mut decoder = h264_decode::Decoder::new(decoder_config);

    let mut frames = decoder
        .decode(input_data)
        .map_err(|e| anyhow::anyhow!("H.264 decode failed: {e}"))?;

    let trailing = decoder
        .flush()
        .map_err(|e| anyhow::anyhow!("H.264 flush failed: {e}"))?;
    frames.extend(trailing);

    let warnings: Vec<String> = decoder
        .take_warnings()
        .into_iter()
        .map(|w| w.to_string())
        .collect();

    Ok((DecodedFrames::H264(frames), warnings))
}

fn decode_h265(input_data: &[u8], verbose: bool) -> Result<(DecodedFrames, Vec<String>)> {
    let decoder_config = h265_decode::DecoderConfig::new()
        .pixel_format(h265_decode::PixelFormat::Yuv420p)
        .collect_warnings(verbose);

    let mut decoder = h265_decode::Decoder::new(decoder_config);

    let mut frames = decoder
        .decode(input_data)
        .map_err(|e| anyhow::anyhow!("H.265 decode failed: {e}"))?;

    let trailing = decoder
        .flush()
        .map_err(|e| anyhow::anyhow!("H.265 flush failed: {e}"))?;
    frames.extend(trailing);

    let warnings: Vec<String> = decoder
        .take_warnings()
        .into_iter()
        .map(|w| w.to_string())
        .collect();

    Ok((DecodedFrames::H265(frames), warnings))
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();

    env_logger::Builder::new()
        .filter_level(if cli.verbose {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        })
        .format_timestamp(None)
        .init();

    // ── Read input ──────────────────────────────────────────────────
    let input_path = &cli.input;
    if !input_path.exists() {
        bail!("input file does not exist: {}", input_path.display());
    }

    let input_data = fs::read(input_path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", input_path.display()))?;

    // ── Detect codec ────────────────────────────────────────────────
    let codec = resolve_codec(cli.codec, input_path, &input_data)?;

    let codec_name = match codec {
        InputCodec::H264 => "H.264",
        InputCodec::H265 => "H.265",
        InputCodec::Auto => unreachable!(),
    };

    eprintln!(
        "input: {} ({} bytes, {codec_name})",
        input_path.display(),
        input_data.len()
    );

    // ── Decode ──────────────────────────────────────────────────────
    let decode_start = Instant::now();

    let (frames, warnings) = match codec {
        InputCodec::H264 => decode_h264(&input_data, cli.verbose)?,
        InputCodec::H265 => decode_h265(&input_data, cli.verbose)?,
        InputCodec::Auto => unreachable!(),
    };

    if frames.is_empty() {
        bail!("no frames decoded from input");
    }

    let width = frames.width();
    let height = frames.height();

    let decode_elapsed = decode_start.elapsed();
    eprintln!(
        "decoded {} frames ({}x{}) in {decode_elapsed:.3?}",
        frames.len(),
        width,
        height,
    );

    // Print decoder warnings if verbose.
    if cli.verbose && !warnings.is_empty() {
        eprintln!("--- decoder warnings ({}) ---", warnings.len());
        for w in &warnings {
            eprintln!("  {w}");
        }
    }

    // ── Configure rav1e ─────────────────────────────────────────────
    let enc_config = EncoderConfig {
        width: width as usize,
        height: height as usize,
        bit_depth: 8,
        chroma_sampling: ChromaSampling::Cs420,
        chroma_sample_position: ChromaSamplePosition::Unknown,
        speed_settings: SpeedSettings::from_preset(cli.speed.min(10)),
        time_base: Rational::new(1, cli.fps as u64),
        min_key_frame_interval: if cli.low_latency { 0 } else { 12 },
        max_key_frame_interval: cli.keyint,
        low_latency: cli.low_latency,
        quantizer: cli.quantizer.min(255),
        min_quantizer: 0,
        bitrate: 0, // quantizer-based rate control
        reservoir_frame_delay: None,
        ..Default::default()
    };

    let rav1e_cfg = Config::new()
        .with_encoder_config(enc_config)
        .with_threads(cli.threads);

    let mut ctx: rav1e::prelude::Context<u8> = rav1e_cfg
        .new_context()
        .map_err(|e| anyhow::anyhow!("failed to create rav1e context: {e}"))?;

    // ── Open output ─────────────────────────────────────────────────
    let output = Output::open(&cli.output)?;

    let mut ivf = IvfWriter::new(output, width as u16, height as u16, 1, cli.fps)
        .map_err(|e| anyhow::anyhow!("failed to write IVF header: {e}"))?;

    // ── Encode loop ─────────────────────────────────────────────────
    let encode_start = Instant::now();

    let total_frames = frames.len();
    let total_input_frames = if cli.max_frames > 0 {
        total_frames.min(cli.max_frames)
    } else {
        total_frames
    };

    let mut frames_sent: u64 = 0;
    let mut packets_received: u64 = 0;
    let mut encoded_bytes: u64 = 0;

    // Create an iterator of codec-agnostic frame references.
    let frame_refs: Vec<DecodedFrameRef<'_>> = match &frames {
        DecodedFrames::H264(v) => v
            .iter()
            .take(total_input_frames)
            .map(DecodedFrameRef::H264)
            .collect(),
        DecodedFrames::H265(v) => v
            .iter()
            .take(total_input_frames)
            .map(DecodedFrameRef::H265)
            .collect(),
    };

    for decoded_frame in &frame_refs {
        let mut rav1e_frame = ctx.new_frame();

        // Copy Y plane.
        rav1e_frame.planes[0].copy_from_raw_u8(decoded_frame.y_data(), decoded_frame.y_stride(), 1);

        // Copy U (Cb) plane.
        rav1e_frame.planes[1].copy_from_raw_u8(decoded_frame.u_data(), decoded_frame.u_stride(), 1);

        // Copy V (Cr) plane.
        rav1e_frame.planes[2].copy_from_raw_u8(decoded_frame.v_data(), decoded_frame.v_stride(), 1);

        match ctx.send_frame(rav1e_frame) {
            Ok(()) => {}
            Err(EncoderStatus::EnoughData) => {
                log::warn!(
                    "encoder returned EnoughData at frame {frames_sent}, draining packets first"
                );
                drain_packets(
                    &mut ctx,
                    &mut ivf,
                    &mut packets_received,
                    &mut encoded_bytes,
                )?;
                // The frame has already been consumed by value above, so we
                // cannot retry.  In practice EnoughData is very rare with
                // sequential send/receive interleaving.
            }
            Err(e) => bail!("rav1e send_frame failed: {e}"),
        }

        frames_sent += 1;

        // Drain any packets that are ready after each send.
        drain_packets(
            &mut ctx,
            &mut ivf,
            &mut packets_received,
            &mut encoded_bytes,
        )?;

        if cli.verbose && frames_sent.is_multiple_of(10) {
            eprintln!(
                "  sent {frames_sent}/{total_input_frames} frames, \
                 received {packets_received} packets"
            );
        }
    }

    // Signal end of input.
    ctx.flush();

    // Drain all remaining packets.
    loop {
        match ctx.receive_packet() {
            Ok(packet) => {
                ivf.write_frame(&packet.data, packet.input_frameno)
                    .context("failed to write IVF frame")?;
                encoded_bytes += packet.data.len() as u64;
                packets_received += 1;
            }
            Err(EncoderStatus::LimitReached) => break,
            Err(EncoderStatus::Encoded) => continue,
            Err(EncoderStatus::NeedMoreData) => break,
            Err(e) => bail!("rav1e receive_packet failed during flush: {e}"),
        }
    }

    // ── Finalise output ─────────────────────────────────────────────
    let output = ivf
        .finish()
        .map_err(|e| anyhow::anyhow!("failed to finalise IVF: {e}"))?;
    output.finish()?;

    let encode_elapsed = encode_start.elapsed();
    let total_elapsed = decode_start.elapsed();
    let encode_fps = if encode_elapsed.as_secs_f64() > 0.0 {
        frames_sent as f64 / encode_elapsed.as_secs_f64()
    } else {
        0.0
    };

    eprintln!();
    eprintln!("--- summary ---");
    eprintln!("  input codec    : {codec_name}");
    eprintln!("  resolution     : {width}x{height}");
    eprintln!("  frames decoded : {total_input_frames}");
    eprintln!("  frames sent    : {frames_sent}");
    eprintln!("  packets out    : {packets_received}");
    eprintln!("  encoded bytes  : {encoded_bytes}");
    eprintln!("  input size     : {} bytes", input_data.len());
    eprintln!(
        "  compression    : {:.1}x",
        if encoded_bytes > 0 {
            input_data.len() as f64 / encoded_bytes as f64
        } else {
            0.0
        }
    );
    eprintln!("  decode time    : {decode_elapsed:.3?}");
    eprintln!("  encode time    : {encode_elapsed:.3?}");
    eprintln!("  total time     : {total_elapsed:.3?}");
    eprintln!("  encode speed   : {encode_fps:.1} fps");
    eprintln!("  speed preset   : {}", cli.speed);
    eprintln!("  quantizer      : {}", cli.quantizer);

    if cli.output.to_str() != Some("-") {
        eprintln!("  output file    : {}", cli.output.display());
    }

    Ok(())
}

/// Drain all currently available packets from the encoder and write them to
/// the IVF output.
fn drain_packets(
    ctx: &mut rav1e::prelude::Context<u8>,
    ivf: &mut IvfWriter<Output>,
    packets_received: &mut u64,
    encoded_bytes: &mut u64,
) -> Result<()> {
    loop {
        match ctx.receive_packet() {
            Ok(packet) => {
                ivf.write_frame(&packet.data, packet.input_frameno)
                    .map_err(|e| anyhow::anyhow!("failed to write IVF frame: {e}"))?;
                *encoded_bytes += packet.data.len() as u64;
                *packets_received += 1;
            }
            Err(EncoderStatus::NeedMoreData | EncoderStatus::Encoded) => break,
            Err(EncoderStatus::LimitReached) => break,
            Err(e) => bail!("rav1e receive_packet failed: {e}"),
        }
    }
    Ok(())
}

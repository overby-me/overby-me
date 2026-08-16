//! CLI tool for decoding H.264 Annex B bytestream files to raw YUV/RGB frames.
//!
//! # Usage
//!
//! ```text
//! h264-decode [OPTIONS] <INPUT>
//!
//! Arguments:
//!   <INPUT>  Path to the input H.264 Annex B file (.264 / .h264)
//!
//! Options:
//!   -o, --output <OUTPUT>    Output file path for raw frames (default: stdout info only)
//!   -f, --format <FORMAT>    Output pixel format: yuv420p, nv12, rgb24, rgba32 [default: yuv420p]
//!   -m, --matrix <MATRIX>    Colour matrix for RGB conversion: bt601, bt709 [default: bt601]
//!       --max-frames <N>     Maximum number of frames to decode (0 = unlimited) [default: 0]
//!       --info               Print stream info (SPS/PPS) and exit without decoding frames
//!   -v, --verbose            Enable verbose logging
//!   -h, --help               Print help
//!   -V, --version            Print version
//! ```

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;

use h264_decode::{DecodedFrame, Decoder, DecoderConfig, PixelFormat};

/// A pure Rust H.264 (AVC) decoder CLI.
///
/// Decodes H.264 Annex B bytestream files into raw YUV or RGB frames.
#[derive(Parser, Debug)]
#[command(name = "h264-decode", version, about)]
struct Cli {
    /// Path to the input H.264 Annex B file (.264 / .h264).
    input: PathBuf,

    /// Output file path for raw decoded frames.
    ///
    /// If omitted, frames are decoded but only summary information is
    /// printed to stderr.  Use `-` to write raw frames to stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Output pixel format.
    #[arg(short, long, default_value = "yuv420p", value_parser = parse_pixel_format)]
    format: PixelFormat,

    /// Colour matrix for YUV→RGB conversion.
    #[arg(short, long, default_value = "bt601", value_parser = parse_colour_matrix)]
    matrix: h264_decode::pixel::ColourMatrix,

    /// Maximum number of frames to decode (0 = unlimited).
    #[arg(long, default_value_t = 0)]
    max_frames: usize,

    /// Print stream information and exit without writing frames.
    #[arg(long)]
    info: bool,

    /// Enable verbose (debug-level) logging.
    #[arg(short, long)]
    verbose: bool,
}

fn parse_pixel_format(s: &str) -> Result<PixelFormat, String> {
    match s.to_lowercase().as_str() {
        "yuv420p" | "i420" => Ok(PixelFormat::Yuv420p),
        "yuv422p" => Ok(PixelFormat::Yuv422p),
        "yuv444p" => Ok(PixelFormat::Yuv444p),
        "nv12" => Ok(PixelFormat::Nv12),
        "rgb24" | "rgb" => Ok(PixelFormat::Rgb24),
        "rgba32" | "rgba" => Ok(PixelFormat::Rgba32),
        other => Err(format!(
            "unknown pixel format '{other}'; expected one of: \
             yuv420p, yuv422p, yuv444p, nv12, rgb24, rgba32"
        )),
    }
}

fn parse_colour_matrix(s: &str) -> Result<h264_decode::pixel::ColourMatrix, String> {
    match s.to_lowercase().as_str() {
        "bt601" | "601" => Ok(h264_decode::pixel::ColourMatrix::Bt601),
        "bt709" | "709" => Ok(h264_decode::pixel::ColourMatrix::Bt709),
        other => Err(format!(
            "unknown colour matrix '{other}'; expected one of: bt601, bt709"
        )),
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Initialise logging.
    env_logger::Builder::new()
        .filter_level(if cli.verbose {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        })
        .format_timestamp(None)
        .init();

    // Read the entire input file.
    let input_path = &cli.input;
    if !input_path.exists() {
        eprintln!("error: input file does not exist: {}", input_path.display());
        std::process::exit(1);
    }

    let input_data = fs::read(input_path).map_err(|e| {
        eprintln!("error: failed to read {}: {e}", input_path.display());
        e
    })?;

    eprintln!(
        "input: {} ({} bytes)",
        input_path.display(),
        input_data.len()
    );

    // Configure the decoder.
    let config = DecoderConfig::new()
        .pixel_format(cli.format)
        .colour_matrix(cli.matrix)
        .collect_warnings(cli.verbose);

    let mut decoder = Decoder::new(config);

    // Open output writer (if requested).
    let mut output_writer: Option<Box<dyn Write>> = match &cli.output {
        Some(path) if path.to_str() == Some("-") => Some(Box::new(io::stdout().lock())),
        Some(path) => {
            let file = fs::File::create(path).map_err(|e| {
                eprintln!("error: cannot create output file {}: {e}", path.display());
                e
            })?;
            Some(Box::new(io::BufWriter::new(file)))
        }
        None => None,
    };

    // Decode.
    let start = Instant::now();
    let mut total_frames: usize = 0;
    let mut total_bytes_out: usize = 0;

    // Feed the entire file in one shot.  The decoder handles incremental
    // Annex B parsing internally.
    let frames = decoder.decode(&input_data)?;

    for frame in &frames {
        if cli.max_frames > 0 && total_frames >= cli.max_frames {
            break;
        }
        total_frames += 1;
        let data = frame.data();
        total_bytes_out += data.len();

        if let Some(ref mut w) = output_writer {
            w.write_all(&data)?;
        }

        print_frame_info(frame, total_frames, cli.verbose);
    }

    // Flush remaining frames.
    if cli.max_frames == 0 || total_frames < cli.max_frames {
        let trailing = decoder.flush()?;
        for frame in &trailing {
            if cli.max_frames > 0 && total_frames >= cli.max_frames {
                break;
            }
            total_frames += 1;
            let data = frame.data();
            total_bytes_out += data.len();

            if let Some(ref mut w) = output_writer {
                w.write_all(&data)?;
            }

            print_frame_info(frame, total_frames, cli.verbose);
        }
    }

    // Print any accumulated warnings.
    if cli.verbose {
        let warnings = decoder.take_warnings();
        if !warnings.is_empty() {
            eprintln!("\n--- warnings ({}) ---", warnings.len());
            for w in &warnings {
                eprintln!("  {w}");
            }
        }
    }

    let elapsed = start.elapsed();
    let fps = if elapsed.as_secs_f64() > 0.0 {
        total_frames as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };

    eprintln!();
    eprintln!("--- summary ---");
    eprintln!("  decoded frames : {total_frames}");
    if decoder.width() > 0 {
        eprintln!(
            "  resolution     : {}x{}",
            decoder.width(),
            decoder.height()
        );
    }
    eprintln!("  output format  : {}", cli.format);
    eprintln!("  output bytes   : {total_bytes_out}");
    eprintln!("  elapsed        : {elapsed:.3?}");
    eprintln!("  throughput     : {fps:.1} fps");

    if let Some(path) = &cli.output
        && path.to_str() != Some("-")
    {
        eprintln!("  output file    : {}", path.display());
    }

    Ok(())
}

fn print_frame_info(frame: &DecodedFrame, index: usize, verbose: bool) {
    if verbose {
        eprintln!(
            "  frame {index:>5}: {}x{} {}{} poc={:<4} fn={:<4} {}",
            frame.width(),
            frame.height(),
            frame
                .picture_type()
                .map(|t| format!("{t}"))
                .unwrap_or_else(|| "?".into()),
            if frame.is_idr() { "(IDR)" } else { "     " },
            frame.pic_order_cnt(),
            frame.frame_num(),
            if frame.is_reference() { "ref" } else { "   " },
        );
    }
}

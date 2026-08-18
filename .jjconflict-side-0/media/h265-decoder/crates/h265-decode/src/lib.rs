// Many modules contain scaffold code for future decoder stages (entropy
// decoding, intra/inter prediction, deblocking, SAO, etc.) that is tested
// but not yet wired into the main decode path.  Suppress dead-code warnings
// crate-wide so that clippy -D warnings passes.
#![allow(dead_code)]

//! # h265-decode
//!
//! A pure Rust H.265/HEVC decoder library.
//!
//! This crate provides a simple API for decoding H.265 (HEVC) bitstreams into
//! raw decoded frames (YUV 4:2:0 planar or converted to RGB), suitable for
//! downstream consumers such as AV1 encoders (e.g. rav1e) or display pipelines.
//!
//! ## Quick start
//!
//! ```no_run
//! use h265_decode::{Decoder, DecoderConfig, PixelFormat};
//!
//! let config = DecoderConfig::new().pixel_format(PixelFormat::Yuv420p);
//! let mut decoder = Decoder::new(config);
//!
//! // Feed raw H.265 Annex B bytestream data
//! let h265_data: &[u8] = &[];
//! let frames = decoder.decode(h265_data).expect("decode failed");
//!
//! for frame in &frames {
//!     println!(
//!         "decoded {}x{} frame, pts={:?}, {} bytes",
//!         frame.width(),
//!         frame.height(),
//!         frame.pts(),
//!         frame.data().len(),
//!     );
//! }
//!
//! // Flush remaining frames at end of stream
//! let trailing = decoder.flush().expect("flush failed");
//! ```
//!
//! ## Architecture
//!
//! The decoder is structured as follows:
//!
//! * **`bitstream`** – Bit-level reader and exp-Golomb decoding, emulation
//!   prevention byte removal.
//! * **`nal`** – HEVC NAL unit header parsing, VPS/SPS/PPS parameter set
//!   parsing (ITU-T H.265 §7.3), and partial slice header parsing.
//! * **`dpb`** – Decoded Picture Buffer with HEVC-style reference picture
//!   set management, reorder bumping, and display-order output.
//! * **`frame`** – `DecodedFrame` and `FramePlane` types representing
//!   decoded pictures with metadata (POC, picture type, reference status).
//! * **`pixel`** – Pixel format definitions, colour matrices (BT.601,
//!   BT.709, BT.2020), and YUV↔RGB conversion utilities.
//! * **`decode`** – Top-level `Decoder` struct that ties everything
//!   together: Annex B demuxing, NAL dispatch, POC derivation, and
//!   scaffold picture reconstruction.
//! * **`error`** – Error and warning types.

pub mod bitstream;
mod decode;
mod dpb;
mod error;
mod frame;
pub mod nal;
pub mod pixel;

pub use decode::{Decoder, DecoderConfig};
pub use error::{DecodeError, DecodeWarning};
pub use frame::{DecodedFrame, FramePlane, PictureType};
pub use pixel::{ColourMatrix, PixelFormat};

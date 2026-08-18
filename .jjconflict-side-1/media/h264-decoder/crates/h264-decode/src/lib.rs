// Many modules contain scaffold code for future decoder stages (entropy
// decoding, intra/inter prediction, deblocking, etc.) that is tested but
// not yet wired into the main decode path.  Suppress dead-code warnings
// crate-wide so that clippy -D warnings passes.
#![allow(dead_code)]

//! # h264-decode
//!
//! A pure Rust H.264 decoder library built on top of [`h264_reader`] for NAL unit parsing.
//!
//! This crate provides a simple API for decoding H.264 (AVC) bitstreams into raw decoded
//! frames (YUV 4:2:0 planar or converted to RGB), suitable for downstream consumers such
//! as AV1 encoders (e.g. rav1e) or display pipelines.
//!
//! ## Quick start
//!
//! ```no_run
//! use h264_decode::{Decoder, DecoderConfig, PixelFormat};
//!
//! let config = DecoderConfig::new().pixel_format(PixelFormat::Yuv420p);
//! let mut decoder = Decoder::new(config);
//!
//! // Feed raw H.264 Annex B bytestream data
//! let h264_data: &[u8] = &[];
//! let frames = decoder.decode(h264_data).expect("decode failed");
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

mod decode;
mod dpb;
mod error;
mod frame;
mod nal_handler;
pub mod pixel;
mod transform;

pub use decode::{Decoder, DecoderConfig};
pub use error::DecodeError;
pub use frame::{DecodedFrame, FramePlane};
pub use pixel::{ColourMatrix, PixelFormat};

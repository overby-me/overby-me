//! Error types for the H.264 decoder.

use std::fmt;

/// Errors that can occur during H.264 decoding.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The input bitstream is malformed or contains invalid syntax.
    #[error("invalid bitstream: {0}")]
    InvalidBitstream(String),

    /// A required Sequence Parameter Set (SPS) was not found.
    #[error("missing SPS with id {0}")]
    MissingSps(u8),

    /// A required Picture Parameter Set (PPS) was not found.
    #[error("missing PPS with id {0}")]
    MissingPps(u8),

    /// The SPS specifies a profile or level that this decoder does not support.
    #[error("unsupported profile/level: {0}")]
    UnsupportedProfile(String),

    /// The bitstream uses a chroma format that is not supported.
    #[error("unsupported chroma format: {0}")]
    UnsupportedChromaFormat(u8),

    /// The bitstream uses a feature that has not been implemented yet.
    #[error("unimplemented feature: {0}")]
    Unimplemented(String),

    /// An error occurred during entropy decoding (CAVLC or CABAC).
    #[error("entropy decoding error: {0}")]
    EntropyDecode(String),

    /// An error occurred during pixel format conversion (e.g. YUV to RGB).
    #[error("pixel conversion error: {0}")]
    PixelConversion(String),

    /// A reference frame required for inter prediction was not found in the DPB.
    #[error("missing reference frame: {0}")]
    MissingReference(String),

    /// The decoded picture buffer is full and cannot accept new frames.
    #[error("DPB overflow: capacity {capacity}, attempted to store frame {frame_num}")]
    DpbOverflow {
        /// Maximum DPB capacity derived from the SPS level.
        capacity: usize,
        /// The frame number that could not be stored.
        frame_num: u32,
    },

    /// A generic I/O error occurred while reading input data.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Convenience alias used throughout the crate.
pub type DecodeResult<T> = Result<T, DecodeError>;

/// Warnings that do not prevent decoding but may indicate data issues.
#[derive(Debug, Clone)]
pub enum DecodeWarning {
    /// A NAL unit was skipped because its type is not handled.
    SkippedNalUnit {
        /// The NAL unit type value that was skipped.
        nal_unit_type: u8,
    },
    /// A corrupted macroblock was concealed rather than decoded.
    ConcealedMacroblock {
        /// Macroblock index within the slice.
        mb_index: usize,
    },
}

impl fmt::Display for DecodeWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeWarning::SkippedNalUnit { nal_unit_type } => {
                write!(f, "skipped NAL unit type {nal_unit_type}")
            }
            DecodeWarning::ConcealedMacroblock { mb_index } => {
                write!(f, "concealed macroblock at index {mb_index}")
            }
        }
    }
}

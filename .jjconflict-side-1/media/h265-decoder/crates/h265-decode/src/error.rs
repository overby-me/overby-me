//! Error types for the H.265/HEVC decoder.

use std::fmt;

/// Errors that can occur during H.265/HEVC decoding.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The input bitstream is malformed or contains invalid syntax.
    #[error("invalid bitstream: {0}")]
    InvalidBitstream(String),

    /// A required Video Parameter Set (VPS) was not found.
    #[error("missing VPS with id {0}")]
    MissingVps(u8),

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

    /// An error occurred during entropy decoding (CABAC).
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

    /// Exp-Golomb or bitstream reader ran out of data.
    #[error("unexpected end of bitstream")]
    EndOfBitstream,

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
    /// A corrupted coding tree unit was concealed rather than decoded.
    ConcealedCtu {
        /// CTU address within the slice.
        ctu_address: usize,
    },
    /// A non-conforming bitstream element was encountered but decoding
    /// continued with a best-effort interpretation.
    NonConformant {
        /// Human-readable description of the issue.
        description: String,
    },
}

impl fmt::Display for DecodeWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeWarning::SkippedNalUnit { nal_unit_type } => {
                write!(f, "skipped NAL unit type {nal_unit_type}")
            }
            DecodeWarning::ConcealedCtu { ctu_address } => {
                write!(f, "concealed CTU at address {ctu_address}")
            }
            DecodeWarning::NonConformant { description } => {
                write!(f, "non-conformant: {description}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_error_display() {
        let err = DecodeError::InvalidBitstream("bad data".into());
        assert_eq!(err.to_string(), "invalid bitstream: bad data");
    }

    #[test]
    fn test_decode_error_missing_vps() {
        let err = DecodeError::MissingVps(0);
        assert_eq!(err.to_string(), "missing VPS with id 0");
    }

    #[test]
    fn test_decode_error_missing_sps() {
        let err = DecodeError::MissingSps(1);
        assert_eq!(err.to_string(), "missing SPS with id 1");
    }

    #[test]
    fn test_decode_error_missing_pps() {
        let err = DecodeError::MissingPps(2);
        assert_eq!(err.to_string(), "missing PPS with id 2");
    }

    #[test]
    fn test_decode_error_dpb_overflow() {
        let err = DecodeError::DpbOverflow {
            capacity: 16,
            frame_num: 17,
        };
        assert!(err.to_string().contains("capacity 16"));
        assert!(err.to_string().contains("frame 17"));
    }

    #[test]
    fn test_decode_error_end_of_bitstream() {
        let err = DecodeError::EndOfBitstream;
        assert_eq!(err.to_string(), "unexpected end of bitstream");
    }

    #[test]
    fn test_decode_warning_display() {
        let w = DecodeWarning::SkippedNalUnit { nal_unit_type: 42 };
        assert_eq!(w.to_string(), "skipped NAL unit type 42");

        let w = DecodeWarning::ConcealedCtu { ctu_address: 7 };
        assert_eq!(w.to_string(), "concealed CTU at address 7");

        let w = DecodeWarning::NonConformant {
            description: "bad slice order".into(),
        };
        assert_eq!(w.to_string(), "non-conformant: bad slice order");
    }
}

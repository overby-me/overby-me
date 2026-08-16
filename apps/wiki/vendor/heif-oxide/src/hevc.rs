//! HEVC glue: `hvcC` configuration parsing and item codestream decoding.
//!
//! A HEIF `hvc1` item stores its VPS/SPS/PPS parameter sets inside the
//! `hvcC` property (HEVCDecoderConfigurationRecord, ISO/IEC 14496-15 §8.3.3)
//! and its slice NAL units as a **length-prefixed** stream in `mdat`. The
//! `rust_h265` decoder consumes Annex B (start-code delimited) NAL units, so
//! this module rebuilds an Annex B stream — parameter sets first, then the
//! item's NALs with each length prefix replaced by a start code — and runs
//! the decoder over it.

use rust_h265::{parse_annex_b, Decoder, Frame};

use crate::bytes::Reader;
use crate::error::HeifError;

/// The parts of an `hvcC` record the decoder needs.
pub struct HvcConfig {
    /// Byte width of each NAL length prefix in the item payload
    /// (`lengthSizeMinusOne + 1`; 1, 2 or 4).
    pub nal_length_size: u8,
    /// Parameter-set NAL units (VPS, SPS, PPS, and any prefix SEI), in
    /// record order, without start codes or length prefixes.
    pub parameter_sets: Vec<Vec<u8>>,
}

/// Parse an HEVCDecoderConfigurationRecord.
pub fn parse_hvcc(record: &[u8]) -> Result<HvcConfig, HeifError> {
    let mut r = Reader::new(record, "hvcC");
    // Fixed-layout preamble (ISO 14496-15 §8.3.3.1):
    //   [0]     configurationVersion (= 1)
    //   [1..21] profile/tier/level, constraint flags, chroma format,
    //           bit depths, frame rate — all irrelevant here, the SPS
    //           carries the authoritative values.
    //   [21]    …(2 bits)… | lengthSizeMinusOne (2 bits)
    //   [22]    numOfArrays
    let version = r.u8()?;
    if version != 1 {
        return Err(HeifError::Invalid(format!(
            "hvcC configurationVersion {version} (expected 1)"
        )));
    }
    r.skip(20)?;
    let nal_length_size = (r.u8()? & 0x3) + 1;
    let num_arrays = r.u8()?;
    let mut parameter_sets = Vec::new();
    for _ in 0..num_arrays {
        // array_completeness (1) | reserved (1) | NAL_unit_type (6)
        let _type_byte = r.u8()?;
        let num_nalus = r.u16()?;
        for _ in 0..num_nalus {
            let len = r.u16()? as usize;
            parameter_sets.push(r.bytes(len)?.to_vec());
        }
    }
    Ok(HvcConfig {
        nal_length_size,
        parameter_sets,
    })
}

/// Convert an `hvcC` config + length-prefixed item payload into one Annex B
/// stream: `startcode paramset ... startcode nal ...`.
pub fn build_annex_b(config: &HvcConfig, item_payload: &[u8]) -> Result<Vec<u8>, HeifError> {
    const START_CODE: [u8; 4] = [0, 0, 0, 1];
    let mut stream = Vec::with_capacity(
        item_payload.len()
            + config
                .parameter_sets
                .iter()
                .map(|p| p.len() + 4)
                .sum::<usize>()
            + 64,
    );
    for ps in &config.parameter_sets {
        stream.extend_from_slice(&START_CODE);
        stream.extend_from_slice(ps);
    }
    let mut r = Reader::new(item_payload, "hvc1 item payload");
    while r.remaining() > 0 {
        let len = match config.nal_length_size {
            1 => r.u8()? as usize,
            2 => r.u16()? as usize,
            4 => r.u32()? as usize,
            n => {
                return Err(HeifError::Invalid(format!(
                    "hvcC NAL length size {n} (expected 1, 2 or 4)"
                )))
            }
        };
        stream.extend_from_slice(&START_CODE);
        stream.extend_from_slice(r.bytes(len)?);
    }
    Ok(stream)
}

/// Decode a single still picture from an Annex B stream, returning the first
/// output frame (a HEIF item is exactly one coded picture).
pub fn decode_first_frame(annex_b: &[u8]) -> Result<Frame, HeifError> {
    let nals = parse_annex_b(annex_b);
    if nals.is_empty() {
        return Err(HeifError::Codec("no NAL units in item payload".into()));
    }
    let mut decoder = Decoder::new();
    for nal in &nals {
        match decoder.decode_nal(nal) {
            Ok(Some(frame)) => return Ok(frame),
            Ok(None) => {}
            Err(e) => return Err(HeifError::Codec(format!("{e:?}"))),
        }
    }
    decoder
        .flush()
        .ok_or_else(|| HeifError::Codec("decoder produced no frame".into()))
}

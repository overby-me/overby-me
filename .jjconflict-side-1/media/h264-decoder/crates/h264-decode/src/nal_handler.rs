//! NAL unit handler bridging [`h264_reader`] parsing to the decoder core.
//!
//! This module provides:
//!
//! - **Parameter-set storage** ([`SpsStore`] / [`PpsStore`]) backed by
//!   h264-reader's own [`SeqParameterSet`] and [`PicParameterSet`] types,
//!   parsed via `from_bits` using `bitstream-io`.
//!
//! - **[`NalAccumulator`]** – collects parsed NAL events (SPS, PPS, slices,
//!   delimiters) that the top-level [`Decoder`](crate::Decoder) drains after
//!   each push of Annex B data.
//!
//! - **[`DecoderNalHandler`]** – implements h264-reader 0.8's
//!   [`AccumulatedNalHandler`] trait so that the push-based Annex B parser
//!   can drive our decoder.
//!
//! - **[`SliceInfo`]** – metadata extracted from a coded slice NAL unit
//!   (slice type, frame_num, PPS reference, raw RBSP bytes).
//!
//! - A small **[`ExpGolombReader`]** for bootstrapping slice-header parsing.

use h264_reader::Context as H264Context;
use h264_reader::nal::pps::PicParameterSet;
use h264_reader::nal::sps::SeqParameterSet;
use h264_reader::nal::{Nal, RefNal, UnitType};
use h264_reader::push::{AccumulatedNalHandler, NalInterest};
use h264_reader::rbsp::BitReader as RbspBitReader;

use std::collections::HashMap;
use std::io::Read;

use crate::error::DecodeError;
use crate::frame::PictureType;

// ---------------------------------------------------------------------------
// Parsed parameter-set storage
// ---------------------------------------------------------------------------

/// Stores active Sequence Parameter Sets keyed by `seq_parameter_set_id`.
#[derive(Debug, Clone, Default)]
pub struct SpsStore {
    sets: HashMap<u32, SeqParameterSet>,
}

impl SpsStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, id: u32, sps: SeqParameterSet) {
        self.sets.insert(id, sps);
    }

    pub fn get(&self, id: u32) -> Option<&SeqParameterSet> {
        self.sets.get(&id)
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    pub fn clear(&mut self) {
        self.sets.clear();
    }
}

/// Stores active Picture Parameter Sets keyed by `pic_parameter_set_id`.
#[derive(Debug, Clone, Default)]
pub struct PpsStore {
    sets: HashMap<u32, PicParameterSet>,
}

impl PpsStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, id: u32, pps: PicParameterSet) {
        self.sets.insert(id, pps);
    }

    pub fn get(&self, id: u32) -> Option<&PicParameterSet> {
        self.sets.get(&id)
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    pub fn clear(&mut self) {
        self.sets.clear();
    }
}

// ---------------------------------------------------------------------------
// Slice metadata extracted during NAL parsing
// ---------------------------------------------------------------------------

/// Information extracted from a single coded slice NAL unit.
///
/// This is produced by the NAL handler after it has parsed the slice header
/// and collected the raw RBSP bytes of the slice data (entropy-coded
/// residual + macroblock layer).  The decoder core consumes this to
/// reconstruct the picture.
#[derive(Debug, Clone)]
pub struct SliceInfo {
    /// The NAL unit type that carried this slice.
    pub nal_unit_type: NalUnitType,
    /// NAL reference IDC (0 = non-reference, >0 = reference).
    pub nal_ref_idc: u8,
    /// Slice type (I / P / B) mapped to our [`PictureType`].
    pub picture_type: PictureType,
    /// `frame_num` from the slice header.
    pub frame_num: u32,
    /// `pic_parameter_set_id` referenced by this slice.
    pub pps_id: u32,
    /// Whether this slice belongs to an IDR picture.
    pub is_idr: bool,
    /// `idr_pic_id` (only meaningful when `is_idr` is true).
    pub idr_pic_id: u32,
    /// Picture order count (Type 0) – `pic_order_cnt_lsb` from slice header.
    pub pic_order_cnt_lsb: Option<u32>,
    /// `delta_pic_order_cnt_bottom` (Type 0, field coding).
    pub delta_pic_order_cnt_bottom: Option<i32>,
    /// `first_mb_in_slice` – address of the first macroblock in this slice.
    pub first_mb_in_slice: u32,
    /// Whether the `field_pic_flag` was set.
    pub field_pic_flag: bool,
    /// Whether the `bottom_field_flag` was set.
    pub bottom_field_flag: bool,
    /// Raw RBSP bytes of the slice data (everything after the slice header).
    ///
    /// The decoder's entropy-decoding stage reads from this buffer.
    pub data: Vec<u8>,
    /// Number of macroblocks wide the picture is (derived from active SPS).
    pub pic_width_in_mbs: u32,
    /// Number of macroblock rows the picture is (derived from active SPS).
    pub pic_height_in_map_units: u32,
    /// Bit depth of the luma samples (default 8).
    pub bit_depth_luma: u8,
    /// Bit depth of the chroma samples (default 8).
    pub bit_depth_chroma: u8,
    /// Chroma format IDC from the active SPS (1 = 4:2:0, 2 = 4:2:2, 3 = 4:4:4).
    pub chroma_format_idc: u8,
    /// QP_Y initial value for this slice: `26 + pic_init_qp_minus26 + slice_qp_delta`.
    pub qp_y: i32,
}

// ---------------------------------------------------------------------------
// NAL unit type mirror (avoids leaking h264-reader enums everywhere)
// ---------------------------------------------------------------------------

/// Simplified NAL unit type enum covering the types the decoder cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NalUnitType {
    /// Non-IDR coded slice (type 1).
    SliceNonIdr,
    /// Coded slice data partition A (type 2).
    SliceDataPartA,
    /// IDR coded slice (type 5).
    SliceIdr,
    /// Supplemental Enhancement Information (type 6).
    Sei,
    /// Sequence Parameter Set (type 7).
    Sps,
    /// Picture Parameter Set (type 8).
    Pps,
    /// Access unit delimiter (type 9).
    AccessUnitDelimiter,
    /// End of sequence (type 10).
    EndOfSequence,
    /// End of stream (type 11).
    EndOfStream,
    /// Any other NAL unit type not specifically handled.
    Other(u8),
}

impl NalUnitType {
    /// Map from a raw NAL unit type byte to our type.
    pub fn from_raw(raw: u8) -> Self {
        match raw {
            1 => NalUnitType::SliceNonIdr,
            2 => NalUnitType::SliceDataPartA,
            5 => NalUnitType::SliceIdr,
            6 => NalUnitType::Sei,
            7 => NalUnitType::Sps,
            8 => NalUnitType::Pps,
            9 => NalUnitType::AccessUnitDelimiter,
            10 => NalUnitType::EndOfSequence,
            11 => NalUnitType::EndOfStream,
            other => NalUnitType::Other(other),
        }
    }

    /// Convert from h264-reader's [`UnitType`].
    pub fn from_h264_reader(ut: UnitType) -> Self {
        Self::from_raw(ut.id())
    }

    /// Returns `true` if this NAL carries coded slice data.
    pub fn is_slice(self) -> bool {
        matches!(
            self,
            NalUnitType::SliceNonIdr | NalUnitType::SliceIdr | NalUnitType::SliceDataPartA
        )
    }

    /// Returns `true` for IDR slices.
    pub fn is_idr(self) -> bool {
        self == NalUnitType::SliceIdr
    }
}

// ---------------------------------------------------------------------------
// NAL accumulator – collects decoded NAL events for the decoder
// ---------------------------------------------------------------------------

/// Events produced by the NAL handler as it processes a bytestream.
#[derive(Debug, Clone)]
pub enum NalEvent {
    /// A new SPS was parsed and stored.
    Sps {
        /// `seq_parameter_set_id`
        id: u32,
    },
    /// A new PPS was parsed and stored.
    Pps {
        /// `pic_parameter_set_id`
        id: u32,
    },
    /// A coded slice was fully received and its header parsed.
    Slice(SliceInfo),
    /// An access-unit delimiter was encountered (marks picture boundary).
    AccessUnitDelimiter,
    /// End of sequence.
    EndOfSequence,
    /// End of stream.
    EndOfStream,
}

/// Accumulates parsed NAL units and makes them available to the decoder.
///
/// After pushing bytes through the `h264_reader` Annex B reader, the
/// decoder drains events from this accumulator to process parameter sets
/// and decode slices.
#[derive(Debug, Default)]
pub struct NalAccumulator {
    /// Ordered list of NAL events produced during the last push.
    events: Vec<NalEvent>,
    /// SPS storage (persists across pushes).
    pub sps_store: SpsStore,
    /// PPS storage (persists across pushes).
    pub pps_store: PpsStore,
    /// h264-reader context for SPS/PPS cross-referencing.
    pub h264_ctx: H264Context,
}

impl NalAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drain all events accumulated since the last drain.
    pub fn drain_events(&mut self) -> Vec<NalEvent> {
        std::mem::take(&mut self.events)
    }

    /// Returns `true` if there are pending events.
    pub fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    /// Reset all state (parameter sets, events, buffers).
    pub fn reset(&mut self) {
        self.events.clear();
        self.sps_store.clear();
        self.pps_store.clear();
        self.h264_ctx = H264Context::default();
    }

    // ------------------------------------------------------------------
    // SPS / PPS parsing using h264-reader 0.8
    // ------------------------------------------------------------------

    /// Parse an SPS from raw RBSP bytes using h264-reader's parser.
    ///
    /// On success the SPS is stored in [`sps_store`](Self::sps_store)
    /// and registered in the h264-reader [`Context`](H264Context),
    /// and the SPS id is returned.
    pub fn parse_sps(&mut self, rbsp: &[u8]) -> Result<u32, DecodeError> {
        if rbsp.is_empty() {
            return Err(DecodeError::InvalidBitstream("empty SPS NAL unit".into()));
        }

        let cursor = std::io::Cursor::new(rbsp);
        let bit_reader = RbspBitReader::new(cursor);

        let sps = SeqParameterSet::from_bits(bit_reader)
            .map_err(|e| DecodeError::InvalidBitstream(format!("SPS parse error: {e:?}")))?;

        let id = sps.seq_parameter_set_id.id() as u32;
        self.h264_ctx.put_seq_param_set(sps.clone());
        self.sps_store.insert(id, sps);

        log::debug!("parsed SPS id={id}");
        self.events.push(NalEvent::Sps { id });
        Ok(id)
    }

    /// Parse a PPS from raw RBSP bytes using h264-reader's parser.
    ///
    /// On success the PPS is stored in [`pps_store`](Self::pps_store)
    /// and registered in the h264-reader [`Context`](H264Context),
    /// and the PPS id is returned.
    pub fn parse_pps(&mut self, rbsp: &[u8]) -> Result<u32, DecodeError> {
        if rbsp.is_empty() {
            return Err(DecodeError::InvalidBitstream("empty PPS NAL unit".into()));
        }

        let cursor = std::io::Cursor::new(rbsp);
        let bit_reader = RbspBitReader::new(cursor);

        let pps = PicParameterSet::from_bits(&self.h264_ctx, bit_reader)
            .map_err(|e| DecodeError::InvalidBitstream(format!("PPS parse error: {e:?}")))?;

        let id = pps.pic_parameter_set_id.id() as u32;
        self.h264_ctx.put_pic_param_set(pps.clone());
        self.pps_store.insert(id, pps);

        log::debug!("parsed PPS id={id}");
        self.events.push(NalEvent::Pps { id });
        Ok(id)
    }

    /// Parse a slice header (minimal bootstrap) and package slice info.
    pub fn parse_slice(
        &mut self,
        nal_type: NalUnitType,
        nal_ref_idc: u8,
        rbsp: &[u8],
    ) -> Result<SliceInfo, DecodeError> {
        if rbsp.is_empty() {
            return Err(DecodeError::InvalidBitstream("empty slice NAL unit".into()));
        }

        // Parse the first few exp-Golomb fields from the slice header:
        //   first_mb_in_slice  (ue)
        //   slice_type         (ue)
        //   pic_parameter_set_id (ue)
        let mut reader = ExpGolombReader::new(rbsp);

        let first_mb_in_slice = reader
            .read_ue()
            .ok_or_else(|| DecodeError::InvalidBitstream("truncated first_mb_in_slice".into()))?;

        let slice_type_raw = reader
            .read_ue()
            .ok_or_else(|| DecodeError::InvalidBitstream("truncated slice_type".into()))?;

        let pps_id = reader
            .read_ue()
            .ok_or_else(|| DecodeError::InvalidBitstream("truncated pps_id".into()))?;

        let picture_type = match slice_type_raw % 5 {
            0 | 5 => PictureType::P,
            1 | 6 => PictureType::B,
            2 | 7 => PictureType::I,
            3 | 8 => PictureType::P, // SP treated as P
            4 | 9 => PictureType::I, // SI treated as I
            _ => {
                return Err(DecodeError::InvalidBitstream(format!(
                    "invalid slice_type {slice_type_raw}"
                )));
            }
        };

        let is_idr = nal_type.is_idr();

        // Default picture dimensions (will be overridden by SPS when available).
        let pic_width_in_mbs = 0;
        let pic_height_in_map_units = 0;

        let info = SliceInfo {
            nal_unit_type: nal_type,
            nal_ref_idc,
            picture_type,
            frame_num: 0,
            pps_id,
            is_idr,
            idr_pic_id: 0,
            pic_order_cnt_lsb: None,
            delta_pic_order_cnt_bottom: None,
            first_mb_in_slice,
            field_pic_flag: false,
            bottom_field_flag: false,
            data: rbsp.to_vec(),
            pic_width_in_mbs,
            pic_height_in_map_units,
            bit_depth_luma: 8,
            bit_depth_chroma: 8,
            chroma_format_idc: 1,
            qp_y: 26,
        };

        self.events.push(NalEvent::Slice(info.clone()));
        Ok(info)
    }

    /// Record an access-unit delimiter event.
    pub fn push_aud(&mut self) {
        self.events.push(NalEvent::AccessUnitDelimiter);
    }

    /// Record an end-of-sequence event.
    pub fn push_eos(&mut self) {
        self.events.push(NalEvent::EndOfSequence);
    }

    /// Record an end-of-stream event.
    pub fn push_eob(&mut self) {
        self.events.push(NalEvent::EndOfStream);
    }
}

// ---------------------------------------------------------------------------
// AccumulatedNalHandler implementation for h264-reader 0.8 push parsing
// ---------------------------------------------------------------------------

/// Handler that implements h264-reader 0.8's [`AccumulatedNalHandler`]
/// trait, forwarding complete NAL units to our [`NalAccumulator`] for
/// processing.
///
/// # Usage
///
/// ```ignore
/// use h264_reader::push::NalAccumulator as H264NalAccumulator;
/// use h264_reader::push::NalInterest;
///
/// let mut acc = NalAccumulator::new();
/// let handler = DecoderNalHandler::new(&mut acc);
/// let mut nal_acc = H264NalAccumulator::new(handler);
///
/// // Feed Annex B data through h264-reader's parser:
/// // nal_acc.push(&annexb_data);
/// ```
pub struct DecoderNalHandler<'a> {
    accumulator: &'a mut NalAccumulator,
}

impl<'a> DecoderNalHandler<'a> {
    pub fn new(accumulator: &'a mut NalAccumulator) -> Self {
        Self { accumulator }
    }
}

impl AccumulatedNalHandler for DecoderNalHandler<'_> {
    fn nal(&mut self, nal: RefNal<'_>) -> NalInterest {
        if !nal.is_complete() {
            // Keep accumulating until the full NAL is buffered.
            return NalInterest::Buffer;
        }

        let header = match nal.header() {
            Ok(h) => h,
            Err(e) => {
                log::warn!("corrupt NAL header: {e:?}");
                return NalInterest::Buffer;
            }
        };

        let unit_type = header.nal_unit_type();
        let nal_ref_idc = header.nal_ref_idc();
        let our_type = NalUnitType::from_h264_reader(unit_type);

        // Collect the full RBSP payload.
        let mut rbsp_bytes = Vec::new();
        if let Err(e) = nal.rbsp_bytes().read_to_end(&mut rbsp_bytes) {
            log::warn!("failed to read RBSP bytes: {e}");
            return NalInterest::Buffer;
        }

        match our_type {
            NalUnitType::Sps => {
                if let Err(e) = self.accumulator.parse_sps(&rbsp_bytes) {
                    log::warn!("failed to parse SPS: {e}");
                }
            }
            NalUnitType::Pps => {
                if let Err(e) = self.accumulator.parse_pps(&rbsp_bytes) {
                    log::warn!("failed to parse PPS: {e}");
                }
            }
            NalUnitType::SliceNonIdr | NalUnitType::SliceIdr | NalUnitType::SliceDataPartA => {
                if let Err(e) = self
                    .accumulator
                    .parse_slice(our_type, nal_ref_idc, &rbsp_bytes)
                {
                    log::warn!("failed to parse slice header: {e}");
                }
            }
            NalUnitType::AccessUnitDelimiter => {
                self.accumulator.push_aud();
            }
            NalUnitType::EndOfSequence => {
                self.accumulator.push_eos();
            }
            NalUnitType::EndOfStream => {
                self.accumulator.push_eob();
            }
            _ => {
                log::debug!(
                    "ignoring NAL unit type {:?} ({} RBSP bytes)",
                    our_type,
                    rbsp_bytes.len()
                );
            }
        }

        NalInterest::Buffer
    }
}

// ---------------------------------------------------------------------------
// Minimal Exp-Golomb reader for slice header bootstrap
// ---------------------------------------------------------------------------

/// A very small, self-contained exponential-Golomb code reader operating
/// on a byte slice at the bit level.
///
/// This is intentionally minimal – it reads just enough of the slice
/// header to extract `first_mb_in_slice`, `slice_type`, and `pps_id`.
/// The full entropy decoding of the slice data is handled elsewhere.
pub(crate) struct ExpGolombReader<'a> {
    data: &'a [u8],
    byte_offset: usize,
    bit_offset: u8, // 0..7, counts from MSB
}

impl<'a> ExpGolombReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_offset: 0,
            bit_offset: 0,
        }
    }

    /// Read a single bit, returning `None` if data is exhausted.
    pub(crate) fn read_bit(&mut self) -> Option<u8> {
        if self.byte_offset >= self.data.len() {
            return None;
        }
        let bit = (self.data[self.byte_offset] >> (7 - self.bit_offset)) & 1;
        self.bit_offset += 1;
        if self.bit_offset == 8 {
            self.bit_offset = 0;
            self.byte_offset += 1;
        }
        Some(bit)
    }

    /// Read `n` bits as a `u32`, MSB first.
    pub(crate) fn read_bits(&mut self, n: u8) -> Option<u32> {
        let mut val = 0u32;
        for _ in 0..n {
            val = (val << 1) | self.read_bit()? as u32;
        }
        Some(val)
    }

    /// Read an unsigned exp-Golomb coded value (`ue(v)`).
    pub(crate) fn read_ue(&mut self) -> Option<u32> {
        // Count leading zeros.
        let mut leading_zeros = 0u8;
        loop {
            let bit = self.read_bit()?;
            if bit == 1 {
                break;
            }
            leading_zeros += 1;
            if leading_zeros > 31 {
                return None; // protect against malformed streams
            }
        }
        if leading_zeros == 0 {
            return Some(0);
        }
        let suffix = self.read_bits(leading_zeros)?;
        Some((1u32 << leading_zeros) - 1 + suffix)
    }

    /// Read a signed exp-Golomb coded value (`se(v)`).
    #[allow(dead_code)]
    pub(crate) fn read_se(&mut self) -> Option<i32> {
        let code = self.read_ue()?;
        let abs_val = code.div_ceil(2);
        if code % 2 == 0 {
            Some(-(abs_val as i32))
        } else {
            Some(abs_val as i32)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- NalUnitType tests --

    #[test]
    fn test_nal_unit_type_from_raw() {
        assert_eq!(NalUnitType::from_raw(1), NalUnitType::SliceNonIdr);
        assert_eq!(NalUnitType::from_raw(5), NalUnitType::SliceIdr);
        assert_eq!(NalUnitType::from_raw(7), NalUnitType::Sps);
        assert_eq!(NalUnitType::from_raw(8), NalUnitType::Pps);
        assert_eq!(NalUnitType::from_raw(9), NalUnitType::AccessUnitDelimiter);
        assert!(matches!(NalUnitType::from_raw(99), NalUnitType::Other(99)));
    }

    #[test]
    fn test_nal_unit_type_from_h264_reader() {
        let ut = UnitType::for_id(7).unwrap();
        assert_eq!(NalUnitType::from_h264_reader(ut), NalUnitType::Sps);

        let ut = UnitType::for_id(5).unwrap();
        assert_eq!(NalUnitType::from_h264_reader(ut), NalUnitType::SliceIdr);
    }

    #[test]
    fn test_nal_unit_type_is_slice() {
        assert!(NalUnitType::SliceNonIdr.is_slice());
        assert!(NalUnitType::SliceIdr.is_slice());
        assert!(NalUnitType::SliceDataPartA.is_slice());
        assert!(!NalUnitType::Sps.is_slice());
        assert!(!NalUnitType::Pps.is_slice());
    }

    #[test]
    fn test_nal_unit_type_is_idr() {
        assert!(NalUnitType::SliceIdr.is_idr());
        assert!(!NalUnitType::SliceNonIdr.is_idr());
    }

    // -- SpsStore / PpsStore tests --

    #[test]
    fn test_sps_store_basic() {
        let store = SpsStore::new();
        assert!(store.is_empty());
        assert!(store.get(0).is_none());
    }

    #[test]
    fn test_pps_store_basic() {
        let store = PpsStore::new();
        assert!(store.is_empty());
        assert!(store.get(0).is_none());
    }

    // -- NalAccumulator tests --

    #[test]
    fn test_nal_accumulator_new() {
        let acc = NalAccumulator::new();
        assert!(!acc.has_events());
    }

    #[test]
    fn test_nal_accumulator_drain_empty() {
        let mut acc = NalAccumulator::new();
        let events = acc.drain_events();
        assert!(events.is_empty());
    }

    #[test]
    fn test_nal_accumulator_reset() {
        let mut acc = NalAccumulator::new();
        acc.push_eob();
        assert!(acc.has_events());
        acc.reset();
        assert!(!acc.has_events());
        assert!(acc.sps_store.is_empty());
        assert!(acc.pps_store.is_empty());
    }

    #[test]
    fn test_nal_accumulator_events() {
        let mut acc = NalAccumulator::new();
        acc.push_aud();
        acc.push_eos();
        acc.push_eob();
        let events = acc.drain_events();
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], NalEvent::AccessUnitDelimiter));
        assert!(matches!(events[1], NalEvent::EndOfSequence));
        assert!(matches!(events[2], NalEvent::EndOfStream));
    }

    // -- ExpGolombReader tests --

    #[test]
    fn test_exp_golomb_read_bit() {
        // 0b10110000 = 0xB0
        let data = [0xB0u8];
        let mut reader = ExpGolombReader::new(&data);
        assert_eq!(reader.read_bit(), Some(1));
        assert_eq!(reader.read_bit(), Some(0));
        assert_eq!(reader.read_bit(), Some(1));
        assert_eq!(reader.read_bit(), Some(1));
        assert_eq!(reader.read_bit(), Some(0));
        assert_eq!(reader.read_bit(), Some(0));
        assert_eq!(reader.read_bit(), Some(0));
        assert_eq!(reader.read_bit(), Some(0));
        assert_eq!(reader.read_bit(), None);
    }

    #[test]
    fn test_exp_golomb_read_bits() {
        // 0b11001010 = 0xCA
        let data = [0xCAu8];
        let mut reader = ExpGolombReader::new(&data);
        assert_eq!(reader.read_bits(4), Some(0b1100));
        assert_eq!(reader.read_bits(4), Some(0b1010));
    }

    #[test]
    fn test_exp_golomb_ue_zero() {
        // ue(0) = '1' => single bit '1' = code 0
        let data = [0x80u8]; // 0b10000000
        let mut reader = ExpGolombReader::new(&data);
        assert_eq!(reader.read_ue(), Some(0));
    }

    #[test]
    fn test_exp_golomb_ue_one() {
        // ue(1) = '010' => leading zero + '1' + '0' suffix = code 1
        let data = [0x40u8]; // 0b01000000
        let mut reader = ExpGolombReader::new(&data);
        assert_eq!(reader.read_ue(), Some(1));
    }

    #[test]
    fn test_exp_golomb_ue_two() {
        // ue(2) = '011' => leading zero + '1' + '1' suffix = code 2
        let data = [0x60u8]; // 0b01100000
        let mut reader = ExpGolombReader::new(&data);
        assert_eq!(reader.read_ue(), Some(2));
    }

    #[test]
    fn test_exp_golomb_ue_three() {
        // ue(3) = '00100' => 2 leading zeros + '1' + '00' suffix = code 3
        let data = [0x20u8]; // 0b00100000
        let mut reader = ExpGolombReader::new(&data);
        assert_eq!(reader.read_ue(), Some(3));
    }

    #[test]
    fn test_exp_golomb_ue_six() {
        // ue(6) = '00111' => 2 leading zeros + '1' + '11' suffix = code 6
        let data = [0x38u8]; // 0b00111000
        let mut reader = ExpGolombReader::new(&data);
        assert_eq!(reader.read_ue(), Some(6));
    }

    #[test]
    fn test_exp_golomb_se() {
        // se uses ue mapping: ue=0 -> se=0, ue=1 -> se=1, ue=2 -> se=-1, ue=3 -> se=2, etc.
        let data = [0x80u8]; // ue(0)
        let mut reader = ExpGolombReader::new(&data);
        assert_eq!(reader.read_se(), Some(0));

        let data = [0x40u8]; // ue(1)
        let mut reader = ExpGolombReader::new(&data);
        assert_eq!(reader.read_se(), Some(1));

        let data = [0x60u8]; // ue(2)
        let mut reader = ExpGolombReader::new(&data);
        assert_eq!(reader.read_se(), Some(-1));

        let data = [0x20u8]; // ue(3)
        let mut reader = ExpGolombReader::new(&data);
        assert_eq!(reader.read_se(), Some(2));
    }

    #[test]
    fn test_exp_golomb_sequential_reads() {
        // Pack ue(0) followed by ue(1) followed by ue(2):
        // '1' + '010' + '011' = 1_010_011_0 = 0b10100110 = 0xA6
        let data = [0xA6u8];
        let mut reader = ExpGolombReader::new(&data);
        assert_eq!(reader.read_ue(), Some(0)); // '1'
        assert_eq!(reader.read_ue(), Some(1)); // '010'
        assert_eq!(reader.read_ue(), Some(2)); // '011'
    }

    #[test]
    fn test_exp_golomb_exhausted_data() {
        let data = [];
        let mut reader = ExpGolombReader::new(&data);
        assert_eq!(reader.read_ue(), None);
    }

    #[test]
    fn test_parse_slice_minimal() {
        let mut acc = NalAccumulator::new();

        // Encode a minimal slice header:
        // first_mb_in_slice = 0 (ue: '1')
        // slice_type = 2 (I-slice, ue: '011')
        // pps_id = 0 (ue: '1')
        // Bits: 1_011_1_000 = 0b10111000 = 0xB8
        let data = [0xB8u8, 0x00];
        let info = acc.parse_slice(NalUnitType::SliceIdr, 3, &data).unwrap();

        assert_eq!(info.first_mb_in_slice, 0);
        assert_eq!(info.picture_type, PictureType::I);
        assert_eq!(info.pps_id, 0);
        assert!(info.is_idr);
        assert_eq!(info.nal_ref_idc, 3);
    }

    #[test]
    fn test_parse_slice_p_type() {
        let mut acc = NalAccumulator::new();

        // first_mb_in_slice = 0 (ue: '1')
        // slice_type = 0 (P-slice, ue: '1')
        // pps_id = 0 (ue: '1')
        // Bits: 1_1_1_00000 = 0b11100000 = 0xE0
        let data = [0xE0u8, 0x00];
        let info = acc.parse_slice(NalUnitType::SliceNonIdr, 2, &data).unwrap();

        assert_eq!(info.first_mb_in_slice, 0);
        assert_eq!(info.picture_type, PictureType::P);
        assert_eq!(info.pps_id, 0);
        assert!(!info.is_idr);
    }

    #[test]
    fn test_parse_slice_empty() {
        let mut acc = NalAccumulator::new();
        let result = acc.parse_slice(NalUnitType::SliceIdr, 3, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_sps_empty() {
        let mut acc = NalAccumulator::new();
        let result = acc.parse_sps(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_pps_empty() {
        let mut acc = NalAccumulator::new();
        let result = acc.parse_pps(&[]);
        assert!(result.is_err());
    }
}

//! Top-level decoder API for H.265/HEVC.
//!
//! [`Decoder`] is the main entry point for decoding H.265 Annex B bytestreams
//! into raw video frames.  It wraps the NAL parsing layer, the decoded picture
//! buffer, and the reconstruction pipeline into a single, easy-to-use
//! interface.
//!
//! # Example
//!
//! ```no_run
//! use h265_decode::{Decoder, DecoderConfig, PixelFormat};
//!
//! let config = DecoderConfig::new().pixel_format(PixelFormat::Yuv420p);
//! let mut decoder = Decoder::new(config);
//!
//! let h265_bytes: &[u8] = &[/* Annex B data */];
//! let frames = decoder.decode(h265_bytes).expect("decode error");
//!
//! for frame in &frames {
//!     println!("{}x{} frame, {} bytes", frame.width(), frame.height(), frame.data().len());
//! }
//!
//! let trailing = decoder.flush().expect("flush error");
//! ```

use crate::bitstream::remove_emulation_prevention;
use crate::dpb::{Dpb, max_dpb_frames};
use crate::error::{DecodeError, DecodeResult, DecodeWarning};
use crate::frame::{DecodedFrame, PictureType};
use crate::nal::{
    NalHeader, NalUnitType, PpsStore, Sps, SpsStore, VpsStore, parse_pps, parse_slice_header,
    parse_sps, parse_vps, sps_cropped_height, sps_cropped_width,
};
use crate::pixel::{ColourMatrix, PixelFormat};

// ---------------------------------------------------------------------------
// DecoderConfig
// ---------------------------------------------------------------------------

/// Configuration for the [`Decoder`].
///
/// Use the builder methods to customise the decoder before constructing it
/// with [`Decoder::new`].
#[derive(Debug, Clone)]
pub struct DecoderConfig {
    /// Desired output pixel format.
    pixel_format: PixelFormat,
    /// Colour matrix for YUV→RGB conversion (only used when the output
    /// format is RGB/RGBA).
    colour_matrix: ColourMatrix,
    /// If `true`, non-fatal warnings are collected and can be retrieved
    /// with [`Decoder::take_warnings`].
    collect_warnings: bool,
    /// Maximum number of reference frames to allow.  `None` means derive
    /// from the SPS level (the normal case).
    max_ref_frames: Option<usize>,
}

impl DecoderConfig {
    /// Create a new default configuration.
    ///
    /// Defaults:
    /// * pixel format: YUV 4:2:0 planar
    /// * colour matrix: BT.709 (the HEVC default for HD content)
    /// * collect warnings: `false`
    /// * max ref frames: derived from SPS
    pub fn new() -> Self {
        Self {
            pixel_format: PixelFormat::Yuv420p,
            colour_matrix: ColourMatrix::Bt709,
            collect_warnings: false,
            max_ref_frames: None,
        }
    }

    /// Set the output pixel format.
    ///
    /// The decoder natively produces YUV 4:2:0 planar; other formats
    /// are converted on the fly.
    pub fn pixel_format(mut self, pf: PixelFormat) -> Self {
        self.pixel_format = pf;
        self
    }

    /// Set the colour matrix for YUV→RGB conversion.
    pub fn colour_matrix(mut self, cm: ColourMatrix) -> Self {
        self.colour_matrix = cm;
        self
    }

    /// Enable or disable warning collection.
    pub fn collect_warnings(mut self, enable: bool) -> Self {
        self.collect_warnings = enable;
        self
    }

    /// Override the maximum number of reference frames.
    ///
    /// Setting `Some(n)` caps the DPB at `n` frames regardless of the
    /// SPS level.  `None` (the default) derives the limit from the SPS.
    pub fn max_ref_frames(mut self, n: Option<usize>) -> Self {
        self.max_ref_frames = n;
        self
    }
}

impl Default for DecoderConfig {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Active parameters (cached from the most recent SPS)
// ---------------------------------------------------------------------------

/// Parameters cached from the active SPS for quick access during slice
/// decoding.
#[derive(Debug, Clone)]
struct ActiveParams {
    /// Cropped output width in luma samples.
    pic_width: u32,
    /// Cropped output height in luma samples.
    pic_height: u32,
    /// Coded width in luma samples (before crop).
    coded_width: u32,
    /// Coded height in luma samples (before crop).
    coded_height: u32,
    /// Chroma format (0=mono, 1=4:2:0, 2=4:2:2, 3=4:4:4).
    chroma_format_idc: u8,
    /// Luma bit depth.
    bit_depth_luma: u8,
    /// Chroma bit depth.
    bit_depth_chroma: u8,
    /// Level indicator (general_level_idc).
    level_idc: u8,
    /// `log2_max_pic_order_cnt_lsb`.
    log2_max_pic_order_cnt_lsb: u8,
    /// Maximum DPB buffering for the highest temporal sub-layer.
    max_dec_pic_buffering: u32,
    /// Maximum reorder frames for the highest temporal sub-layer.
    max_num_reorder_pics: u32,
    /// Active SPS ID.
    sps_id: u32,
}

impl Default for ActiveParams {
    fn default() -> Self {
        Self {
            pic_width: 0,
            pic_height: 0,
            coded_width: 0,
            coded_height: 0,
            chroma_format_idc: 1,
            bit_depth_luma: 8,
            bit_depth_chroma: 8,
            level_idc: 0,
            log2_max_pic_order_cnt_lsb: 8,
            max_dec_pic_buffering: 1,
            max_num_reorder_pics: 0,
            sps_id: 0,
        }
    }
}

fn active_params_from_sps(sps: &Sps) -> ActiveParams {
    let highest_tid = sps.max_sub_layers_minus1 as usize;
    ActiveParams {
        pic_width: sps_cropped_width(sps),
        pic_height: sps_cropped_height(sps),
        coded_width: sps.pic_width_in_luma_samples,
        coded_height: sps.pic_height_in_luma_samples,
        chroma_format_idc: sps.chroma_format_idc,
        bit_depth_luma: sps.bit_depth_luma,
        bit_depth_chroma: sps.bit_depth_chroma,
        level_idc: sps.profile_tier_level.general_level_idc,
        log2_max_pic_order_cnt_lsb: sps.log2_max_pic_order_cnt_lsb,
        max_dec_pic_buffering: sps
            .max_dec_pic_buffering_minus1
            .get(highest_tid)
            .copied()
            .unwrap_or(0)
            + 1,
        max_num_reorder_pics: sps
            .max_num_reorder_pics
            .get(highest_tid)
            .copied()
            .unwrap_or(0),
        sps_id: sps.sps_id,
    }
}

// ---------------------------------------------------------------------------
// POC state
// ---------------------------------------------------------------------------

/// State for picture order count derivation (§8.3.1).
///
/// HEVC uses a single POC type based on `pic_order_cnt_lsb` and a running
/// MSB accumulator, similar to H.264 POC type 0.
#[derive(Debug, Clone, Default)]
struct PocState {
    /// POC MSB of the previous reference picture.
    prev_poc_msb: i32,
    /// POC LSB of the previous reference picture.
    prev_poc_lsb: u32,
    /// Most recently computed full POC.
    prev_poc: i32,
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// The main H.265/HEVC decoder.
///
/// Feed Annex B bytestream data via [`Decoder::decode`] and collect decoded
/// frames.  Call [`Decoder::flush`] at end-of-stream to retrieve any
/// remaining pictures held in the DPB for reordering.
///
/// # Thread safety
///
/// The decoder is `Send` but not `Sync`.  It maintains internal mutable
/// state (DPB, parameter sets, Annex B buffer) and is designed for
/// single-threaded use.
pub struct Decoder {
    /// User configuration.
    config: DecoderConfig,
    /// Parameter set stores.
    vps_store: VpsStore,
    sps_store: SpsStore,
    pps_store: PpsStore,
    /// Decoded Picture Buffer.
    dpb: Dpb,
    /// Cached active parameters (from the most recent SPS).
    active_params: ActiveParams,
    /// Total frames decoded so far (decode order counter).
    decode_frame_counter: u64,
    /// POC derivation state.
    poc_state: PocState,
    /// Accumulated warnings.
    warnings: Vec<DecodeWarning>,
    /// Annex B byte buffer for incremental NAL extraction.
    annex_b_buffer: Vec<u8>,
}

impl Decoder {
    /// Create a new decoder with the given configuration.
    pub fn new(config: DecoderConfig) -> Self {
        Self {
            config,
            vps_store: VpsStore::new(),
            sps_store: SpsStore::new(),
            pps_store: PpsStore::new(),
            dpb: Dpb::new(),
            active_params: ActiveParams::default(),
            decode_frame_counter: 0,
            poc_state: PocState::default(),
            warnings: Vec::new(),
            annex_b_buffer: Vec::new(),
        }
    }

    /// Create a decoder with the default configuration.
    pub fn with_defaults() -> Self {
        Self::new(DecoderConfig::default())
    }

    /// Returns a reference to the active configuration.
    #[inline]
    pub fn config(&self) -> &DecoderConfig {
        &self.config
    }

    /// Current picture width in luma pixels, or 0 if no SPS has been
    /// parsed yet.
    #[inline]
    pub fn width(&self) -> u32 {
        self.active_params.pic_width
    }

    /// Current picture height in luma pixels, or 0 if no SPS has been
    /// parsed yet.
    #[inline]
    pub fn height(&self) -> u32 {
        self.active_params.pic_height
    }

    /// Number of frames currently buffered in the DPB.
    #[inline]
    pub fn buffered_frames(&self) -> usize {
        self.dpb.len()
    }

    /// Total number of frames decoded so far (in decode order).
    #[inline]
    pub fn decoded_frame_count(&self) -> u64 {
        self.decode_frame_counter
    }

    /// Take any accumulated warnings, leaving the internal list empty.
    pub fn take_warnings(&mut self) -> Vec<DecodeWarning> {
        std::mem::take(&mut self.warnings)
    }

    // ------------------------------------------------------------------
    // Public decode API
    // ------------------------------------------------------------------

    /// Feed raw H.265 Annex B bytestream data into the decoder.
    ///
    /// The input `data` does not need to be aligned to NAL unit boundaries –
    /// the decoder maintains an internal buffer and handles incremental
    /// parsing.
    ///
    /// Returns a (possibly empty) list of decoded frames that are ready for
    /// display, in display order (sorted by picture order count).
    pub fn decode(&mut self, data: &[u8]) -> DecodeResult<Vec<DecodedFrame>> {
        // Append incoming data and extract complete NAL units.
        self.annex_b_buffer.extend_from_slice(data);
        let nal_units = self.extract_nal_units();

        let mut output_frames = Vec::new();

        for nal_data in nal_units {
            if nal_data.len() < 2 {
                continue;
            }

            // Parse the 2-byte NAL header.
            let header = match NalHeader::parse(&nal_data) {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("failed to parse NAL header: {e}");
                    continue;
                }
            };

            let nal_type = header.nal_type();

            // RBSP is everything after the 2-byte header, with emulation
            // prevention bytes removed.
            let rbsp = remove_emulation_prevention(&nal_data[2..]);

            match nal_type {
                NalUnitType::Vps => {
                    if let Err(e) = self.handle_vps(&rbsp) {
                        log::warn!("VPS parse error: {e}");
                        if self.config.collect_warnings {
                            self.warnings.push(DecodeWarning::NonConformant {
                                description: format!("VPS parse failed: {e}"),
                            });
                        }
                    }
                }
                NalUnitType::Sps => {
                    if let Err(e) = self.handle_sps(&rbsp) {
                        log::warn!("SPS parse error: {e}");
                        if self.config.collect_warnings {
                            self.warnings.push(DecodeWarning::NonConformant {
                                description: format!("SPS parse failed: {e}"),
                            });
                        }
                    }
                }
                NalUnitType::Pps => {
                    if let Err(e) = self.handle_pps(&rbsp) {
                        log::warn!("PPS parse error: {e}");
                        if self.config.collect_warnings {
                            self.warnings.push(DecodeWarning::NonConformant {
                                description: format!("PPS parse failed: {e}"),
                            });
                        }
                    }
                }
                t if t.is_vcl() => match self.handle_slice(&rbsp, t) {
                    Ok(frames) => output_frames.extend(frames),
                    Err(e) => {
                        log::warn!("slice decode error: {e}");
                        if self.config.collect_warnings {
                            self.warnings.push(DecodeWarning::NonConformant {
                                description: format!("slice error: {e}"),
                            });
                        }
                    }
                },
                NalUnitType::Aud => {
                    log::trace!("access unit delimiter");
                }
                NalUnitType::SeiPrefix | NalUnitType::SeiSuffix => {
                    log::trace!("SEI NAL unit ({} bytes)", rbsp.len());
                }
                NalUnitType::EosNut => {
                    let flushed = self.dpb.flush();
                    output_frames.extend(self.convert_output_format(flushed));
                }
                NalUnitType::EobNut => {
                    let flushed = self.dpb.flush();
                    output_frames.extend(self.convert_output_format(flushed));
                }
                NalUnitType::FdNut => {
                    log::trace!("filler data NAL unit");
                }
                _ => {
                    if self.config.collect_warnings {
                        self.warnings.push(DecodeWarning::SkippedNalUnit {
                            nal_unit_type: nal_type.raw(),
                        });
                    }
                    log::debug!("skipping NAL unit type {nal_type}");
                }
            }
        }

        Ok(output_frames)
    }

    /// Flush all buffered frames from the decoder.
    ///
    /// Call this at end-of-stream to retrieve any remaining frames that
    /// were held in the DPB for reordering.  After flushing, the decoder
    /// can still accept new data via [`Decoder::decode`].
    pub fn flush(&mut self) -> DecodeResult<Vec<DecodedFrame>> {
        let flushed = self.dpb.flush();
        Ok(self.convert_output_format(flushed))
    }

    /// Hard-reset the decoder to its initial state, discarding all
    /// buffered data and parameter sets.
    pub fn reset(&mut self) {
        self.vps_store.clear();
        self.sps_store.clear();
        self.pps_store.clear();
        self.dpb.clear();
        self.active_params = ActiveParams::default();
        self.decode_frame_counter = 0;
        self.poc_state = PocState::default();
        self.warnings.clear();
        self.annex_b_buffer.clear();
    }

    // ------------------------------------------------------------------
    // Annex B start-code scanner
    // ------------------------------------------------------------------

    /// Extract complete NAL units from the Annex B buffer.
    ///
    /// Scans for start codes (`0x000001` or `0x00000001`) and splits the
    /// buffer into individual NAL units.  Incomplete data (no trailing
    /// start code yet) is kept in the buffer for the next call.
    fn extract_nal_units(&mut self) -> Vec<Vec<u8>> {
        let mut nals = Vec::new();
        let buf = &self.annex_b_buffer;
        let len = buf.len();

        if len < 4 {
            return nals;
        }

        // Find all start-code positions (byte index of first payload byte).
        let mut positions = Vec::new();
        let mut i = 0;
        while i + 2 < len {
            if buf[i] == 0x00 && buf[i + 1] == 0x00 {
                if buf[i + 2] == 0x01 {
                    positions.push(i + 3);
                    i += 3;
                    continue;
                } else if i + 3 < len && buf[i + 2] == 0x00 && buf[i + 3] == 0x01 {
                    positions.push(i + 4);
                    i += 4;
                    continue;
                }
            }
            i += 1;
        }

        if positions.len() < 2 {
            // Need at least two start codes to delimit one complete NAL.
            return nals;
        }

        // Extract NAL units between consecutive start codes.
        for window in positions.windows(2) {
            let start = window[0];
            let end = window[1];

            // Find the beginning of the *next* start code so we know where
            // the current NAL's payload ends.
            let mut nal_data_end;
            if end >= 4
                && buf[end - 4] == 0
                && buf[end - 3] == 0
                && buf[end - 2] == 0
                && buf[end - 1] == 1
            {
                nal_data_end = end - 4;
            } else if end >= 3 && buf[end - 3] == 0 && buf[end - 2] == 0 && buf[end - 1] == 1 {
                nal_data_end = end - 3;
            } else {
                nal_data_end = end;
            }

            // Trim trailing zeros (some streams pad between NALs).
            while nal_data_end > start && buf[nal_data_end - 1] == 0x00 {
                nal_data_end -= 1;
            }

            if nal_data_end > start {
                nals.push(buf[start..nal_data_end].to_vec());
            }
        }

        // Keep the data from the last start code onward (incomplete NAL).
        let last_pos = *positions.last().unwrap();
        let keep_from = if last_pos >= 4
            && buf[last_pos - 4] == 0
            && buf[last_pos - 3] == 0
            && buf[last_pos - 2] == 0
            && buf[last_pos - 1] == 1
        {
            last_pos - 4
        } else if last_pos >= 3
            && buf[last_pos - 3] == 0
            && buf[last_pos - 2] == 0
            && buf[last_pos - 1] == 1
        {
            last_pos - 3
        } else {
            last_pos
        };

        self.annex_b_buffer = buf[keep_from..].to_vec();

        nals
    }

    // ------------------------------------------------------------------
    // NAL unit handlers
    // ------------------------------------------------------------------

    /// Handle a VPS NAL unit.
    fn handle_vps(&mut self, rbsp: &[u8]) -> DecodeResult<()> {
        let vps = parse_vps(rbsp)?;

        log::info!(
            "VPS id={}: max_layers={}, max_sub_layers={}, level={}",
            vps.vps_id,
            vps.max_layers_minus1 + 1,
            vps.max_sub_layers_minus1 + 1,
            vps.profile_tier_level.general_level_idc,
        );

        self.vps_store.insert(vps.vps_id, vps);
        Ok(())
    }

    /// Handle an SPS NAL unit.
    fn handle_sps(&mut self, rbsp: &[u8]) -> DecodeResult<()> {
        let sps = parse_sps(rbsp)?;

        let params = active_params_from_sps(&sps);

        log::info!(
            "SPS id={}: profile={}, level={}, {}x{} (coded {}x{}), \
             chroma={}, bit_depth={}/{}, poc_lsb_bits={}, dpb_buf={}, reorder={}",
            sps.sps_id,
            sps.profile_tier_level.general_profile_idc,
            params.level_idc,
            params.pic_width,
            params.pic_height,
            params.coded_width,
            params.coded_height,
            params.chroma_format_idc,
            params.bit_depth_luma,
            params.bit_depth_chroma,
            params.log2_max_pic_order_cnt_lsb,
            params.max_dec_pic_buffering,
            params.max_num_reorder_pics,
        );

        // Resize the DPB.
        let dpb_size = self.config.max_ref_frames.unwrap_or_else(|| {
            let pic_size = params.coded_width as u64 * params.coded_height as u64;
            max_dpb_frames(params.level_idc, pic_size, params.max_dec_pic_buffering)
        });

        let bumped = self.dpb.set_max_size(dpb_size);
        if !bumped.is_empty() {
            log::debug!("SPS change bumped {} frames from DPB", bumped.len());
        }
        self.dpb
            .set_max_num_reorder_pics(params.max_num_reorder_pics as usize);

        self.sps_store.insert(sps.sps_id, sps);
        self.active_params = params;

        Ok(())
    }

    /// Handle a PPS NAL unit.
    fn handle_pps(&mut self, rbsp: &[u8]) -> DecodeResult<()> {
        let pps = parse_pps(rbsp)?;

        log::debug!(
            "PPS id={} (sps_id={}): init_qp={}, tiles={}, entropy_sync={}",
            pps.pps_id,
            pps.sps_id,
            pps.init_qp_minus26 + 26,
            pps.tiles_enabled_flag,
            pps.entropy_coding_sync_enabled_flag,
        );

        self.pps_store.insert(pps.pps_id, pps);
        Ok(())
    }

    /// Handle a coded slice NAL unit.
    fn handle_slice(
        &mut self,
        rbsp: &[u8],
        nal_type: NalUnitType,
    ) -> DecodeResult<Vec<DecodedFrame>> {
        if rbsp.is_empty() {
            return Err(DecodeError::InvalidBitstream("empty slice NAL unit".into()));
        }

        // We need the active SPS and a PPS to parse the slice header.
        // Peek at the slice header to find the PPS ID.  The first bit is
        // `first_slice_segment_in_pic_flag`, and for IRAP there is
        // `no_output_of_prior_pics_flag`, then `slice_pic_parameter_set_id`
        // as ue(v).  We need to pre-parse enough to find the PPS.
        let pps_id = self.peek_slice_pps_id(rbsp, nal_type)?;

        let pps = self
            .pps_store
            .get(pps_id)
            .ok_or(DecodeError::MissingPps(pps_id as u8))?
            .clone();

        let sps = self
            .sps_store
            .get(pps.sps_id)
            .ok_or(DecodeError::MissingSps(pps.sps_id as u8))?
            .clone();

        // Update active params if the SPS changed.
        if pps.sps_id != self.active_params.sps_id || self.active_params.pic_width == 0 {
            let params = active_params_from_sps(&sps);
            self.active_params = params;
        }

        let slice_header = parse_slice_header(rbsp, nal_type, &sps, &pps)?;

        if slice_header.dependent_slice_segment_flag {
            // Dependent slice segments share the independent slice header.
            // For the scaffold, we skip them.
            log::trace!("skipping dependent slice segment");
            return Ok(Vec::new());
        }

        // Only process the first slice segment in each picture.
        if !slice_header.first_slice_segment_in_pic_flag {
            log::trace!("skipping non-first slice segment");
            return Ok(Vec::new());
        }

        let is_irap = nal_type.is_irap();
        let is_idr = nal_type.is_idr();
        let is_reference = nal_type.is_reference();

        let picture_type = match slice_header.slice_type {
            0 => PictureType::B,
            1 => PictureType::P,
            2 => PictureType::I,
            _ => PictureType::I,
        };

        // ── Compute POC ─────────────────────────────────────────────
        let poc = self.compute_poc(slice_header.pic_order_cnt_lsb, is_idr);

        let params = &self.active_params;
        let pic_width = params.pic_width;
        let pic_height = params.pic_height;

        if pic_width == 0 || pic_height == 0 {
            return Err(DecodeError::InvalidBitstream(
                "slice received before SPS (no picture dimensions)".into(),
            ));
        }

        log::debug!(
            "slice: type={picture_type}, poc={poc}, nal={nal_type}, \
             irap={is_irap}, idr={is_idr}, ref={is_reference}, \
             {pic_width}x{pic_height}"
        );

        // ── Apply reference picture set ──────────────────────────────
        //
        // In a complete HEVC decoder, we would parse the full RPS from the
        // slice header and mark pictures in the DPB accordingly.  For the
        // scaffold, IDR pictures clear all references, and for other IRAP
        // types we mark all references as unused (since we don't have real
        // inter prediction yet).
        if is_idr {
            self.dpb.mark_all_unused();
        }

        // ── Reconstruct the picture ─────────────────────────────────
        //
        // In a complete H.265 decoder this is where we would:
        //   1. Parse the coding tree unit (CTU) layer via CABAC.
        //   2. Perform intra prediction (planar, DC, angular modes) or
        //      motion-compensated inter prediction for each coding unit.
        //   3. Inverse-transform (DST/DCT) and dequantize the residual.
        //   4. Add the residual to the prediction to reconstruct the
        //      picture.
        //   5. Apply the in-loop filters: deblocking, SAO, and (for
        //      HEVC-SCC or extensions) ALF.
        //
        // The infrastructure for these steps is being built out.  For now
        // we produce a placeholder frame (grey for I-frames, copied from
        // the closest reference for P/B-frames) so that the API contract
        // is fulfilled and downstream consumers can exercise the pipeline.

        let frame = self.reconstruct_picture(
            pic_width,
            pic_height,
            poc,
            picture_type,
            is_irap,
            is_reference,
        )?;

        self.decode_frame_counter += 1;

        // Insert into DPB.
        let dpb_output = self.dpb.insert(frame)?;

        Ok(self.convert_output_format(dpb_output))
    }

    /// Peek at the `slice_pic_parameter_set_id` without consuming the
    /// full slice header.
    fn peek_slice_pps_id(&self, rbsp: &[u8], nal_type: NalUnitType) -> DecodeResult<u32> {
        use crate::bitstream::BitstreamReader;

        let mut r = BitstreamReader::new(rbsp);

        // first_slice_segment_in_pic_flag – u(1)
        let _first_flag = r.read_flag()?;

        // no_output_of_prior_pics_flag – u(1) (only for IRAP)
        if nal_type.is_irap() {
            r.read_flag()?;
        }

        // slice_pic_parameter_set_id – ue(v)
        let pps_id = r.read_ue()?;

        Ok(pps_id)
    }

    // ------------------------------------------------------------------
    // POC computation (§8.3.1)
    // ------------------------------------------------------------------

    /// Compute the picture order count for the current picture.
    ///
    /// HEVC derives POC from `pic_order_cnt_lsb` and a running MSB
    /// accumulator, identical to H.264 POC type 0.
    fn compute_poc(&mut self, poc_lsb: u32, is_idr: bool) -> i32 {
        let max_poc_lsb = 1u32 << self.active_params.log2_max_pic_order_cnt_lsb;

        if is_idr {
            self.poc_state.prev_poc_msb = 0;
            self.poc_state.prev_poc_lsb = 0;
        }

        let prev_msb = self.poc_state.prev_poc_msb;
        let prev_lsb = self.poc_state.prev_poc_lsb;

        let poc_msb = if poc_lsb < prev_lsb && (prev_lsb.wrapping_sub(poc_lsb)) >= max_poc_lsb / 2 {
            prev_msb + max_poc_lsb as i32
        } else if poc_lsb > prev_lsb && (poc_lsb.wrapping_sub(prev_lsb)) > max_poc_lsb / 2 {
            prev_msb - max_poc_lsb as i32
        } else {
            prev_msb
        };

        let poc = poc_msb + poc_lsb as i32;

        // Update state for the next picture.
        self.poc_state.prev_poc_msb = poc_msb;
        self.poc_state.prev_poc_lsb = poc_lsb;
        self.poc_state.prev_poc = poc;

        poc
    }

    // ------------------------------------------------------------------
    // Picture reconstruction (scaffold)
    // ------------------------------------------------------------------

    /// Reconstruct a decoded picture.
    ///
    /// In the current scaffold this produces a YUV 4:2:0 frame.  For
    /// I-frames a mid-grey plane is generated; for P/B-frames the first
    /// available reference is copied (or grey if no reference exists).
    fn reconstruct_picture(
        &self,
        width: u32,
        height: u32,
        poc: i32,
        picture_type: PictureType,
        is_irap: bool,
        is_reference: bool,
    ) -> DecodeResult<DecodedFrame> {
        let luma_size = (width * height) as usize;
        let chroma_w = width.div_ceil(2) as usize;
        let chroma_h = height.div_ceil(2) as usize;
        let chroma_size = chroma_w * chroma_h;

        let (y_data, u_data, v_data) = match picture_type {
            PictureType::I => {
                // Intra: grey (Y=128, Cb=Cr=128 = neutral chroma).
                (
                    vec![128u8; luma_size],
                    vec![128u8; chroma_size],
                    vec![128u8; chroma_size],
                )
            }
            PictureType::P | PictureType::B => {
                // Try to copy from the first available reference frame.
                let ref_list = self.dpb.build_ref_list_0(poc);
                if let Some(reference) = ref_list.first() {
                    if reference.pixel_format().is_planar_yuv()
                        && reference.width() == width
                        && reference.height() == height
                    {
                        (
                            reference.y_plane().data().to_vec(),
                            reference.u_plane().data().to_vec(),
                            reference.v_plane().data().to_vec(),
                        )
                    } else {
                        (
                            vec![128u8; luma_size],
                            vec![128u8; chroma_size],
                            vec![128u8; chroma_size],
                        )
                    }
                } else {
                    (
                        vec![128u8; luma_size],
                        vec![128u8; chroma_size],
                        vec![128u8; chroma_size],
                    )
                }
            }
        };

        let frame = DecodedFrame::from_yuv420p(y_data, u_data, v_data, width, height)
            .with_frame_num(self.decode_frame_counter as u32)
            .with_pic_order_cnt(poc)
            .with_is_reference(is_reference)
            .with_is_irap(is_irap)
            .with_picture_type(picture_type)
            .with_colour_matrix(self.config.colour_matrix);

        Ok(frame)
    }

    // ------------------------------------------------------------------
    // Output format conversion
    // ------------------------------------------------------------------

    /// Convert a batch of DPB-output frames to the configured pixel format.
    fn convert_output_format(&self, frames: Vec<DecodedFrame>) -> Vec<DecodedFrame> {
        if frames.is_empty() {
            return frames;
        }

        match self.config.pixel_format {
            PixelFormat::Yuv420p => frames, // native format, no conversion
            PixelFormat::Rgb24 => frames.into_iter().map(|f| f.to_rgb24()).collect(),
            PixelFormat::Rgba32 => frames.into_iter().map(|f| f.to_rgba32()).collect(),
            PixelFormat::Nv12 => frames.into_iter().map(|f| f.to_nv12()).collect(),
            _ => {
                log::warn!(
                    "output pixel format {:?} not yet supported, returning YUV 4:2:0",
                    self.config.pixel_format
                );
                frames
            }
        }
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new(DecoderConfig::default())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decoder_config_defaults() {
        let config = DecoderConfig::new();
        assert_eq!(config.pixel_format, PixelFormat::Yuv420p);
        assert_eq!(config.colour_matrix, ColourMatrix::Bt709);
        assert!(!config.collect_warnings);
        assert!(config.max_ref_frames.is_none());
    }

    #[test]
    fn test_decoder_config_builder() {
        let config = DecoderConfig::new()
            .pixel_format(PixelFormat::Rgb24)
            .colour_matrix(ColourMatrix::Bt601)
            .collect_warnings(true)
            .max_ref_frames(Some(4));

        assert_eq!(config.pixel_format, PixelFormat::Rgb24);
        assert_eq!(config.colour_matrix, ColourMatrix::Bt601);
        assert!(config.collect_warnings);
        assert_eq!(config.max_ref_frames, Some(4));
    }

    #[test]
    fn test_decoder_new() {
        let decoder = Decoder::with_defaults();
        assert_eq!(decoder.width(), 0);
        assert_eq!(decoder.height(), 0);
        assert_eq!(decoder.buffered_frames(), 0);
        assert_eq!(decoder.decoded_frame_count(), 0);
    }

    #[test]
    fn test_decoder_reset() {
        let mut decoder = Decoder::with_defaults();
        decoder.decode_frame_counter = 42;
        decoder.active_params.pic_width = 1920;
        decoder.reset();
        assert_eq!(decoder.decoded_frame_count(), 0);
        assert_eq!(decoder.width(), 0);
    }

    #[test]
    fn test_decoder_flush_empty() {
        let mut decoder = Decoder::with_defaults();
        let frames = decoder.flush().unwrap();
        assert!(frames.is_empty());
    }

    #[test]
    fn test_decoder_decode_empty() {
        let mut decoder = Decoder::with_defaults();
        let frames = decoder.decode(&[]).unwrap();
        assert!(frames.is_empty());
    }

    #[test]
    fn test_decoder_decode_garbage() {
        let mut decoder = Decoder::new(DecoderConfig::new().collect_warnings(true));
        // Feed random data that won't form valid NAL units.
        let garbage = vec![0xAA; 100];
        let frames = decoder.decode(&garbage).unwrap();
        assert!(frames.is_empty());
    }

    #[test]
    fn test_decoder_warnings_collection() {
        let mut decoder = Decoder::new(DecoderConfig::new().collect_warnings(true));
        assert!(decoder.take_warnings().is_empty());
        // Feed a NAL unit with an unknown type to trigger a warning.
        // Build an Annex B NAL: start code + 2-byte header (type=63=Other)
        // type=63=0b111111, byte0 = 0_111111_0 = 0x7E, byte1 = 00000_001 = 0x01
        let data = [
            0x00, 0x00, 0x00, 0x01, // start code
            0x7E, 0x01, // NAL header: type 63
            0xAA, // payload
            0x00, 0x00, 0x00, 0x01, // next start code (to delimit)
            0x7E, 0x01, // another NAL
            0xBB,
        ];
        let _ = decoder.decode(&data);
        let warnings = decoder.take_warnings();
        assert!(!warnings.is_empty());
    }

    #[test]
    fn test_annex_b_extract_single_nal() {
        let mut decoder = Decoder::with_defaults();

        // Two start codes delimiting one NAL unit.
        // VPS NAL type = 32, byte0 = 0_100000_0 = 0x40, byte1 = 00000_001 = 0x01
        let data = [
            0x00, 0x00, 0x00, 0x01, // start code 1
            0x40, 0x01, // VPS header
            0xAB, 0xCD, // payload
            0x00, 0x00, 0x00, 0x01, // start code 2
            0x40, 0x01, // next NAL (kept in buffer)
            0xEF,
        ];

        decoder.annex_b_buffer.extend_from_slice(&data);
        let nals = decoder.extract_nal_units();

        assert_eq!(nals.len(), 1);
        assert_eq!(nals[0].len(), 4); // 0x40, 0x01, 0xAB, 0xCD
        assert_eq!(nals[0][0], 0x40);
        assert_eq!(nals[0][1], 0x01);
        assert_eq!(nals[0][2], 0xAB);
        assert_eq!(nals[0][3], 0xCD);
    }

    #[test]
    fn test_annex_b_extract_three_byte_start_code() {
        let mut decoder = Decoder::with_defaults();

        let data = [
            0x00, 0x00, 0x01, // 3-byte start code
            0x44, 0x01, // PPS header
            0x12, 0x00, 0x00, 0x01, // 3-byte start code
            0x44, 0x01, // next NAL
            0x34,
        ];

        decoder.annex_b_buffer.extend_from_slice(&data);
        let nals = decoder.extract_nal_units();

        assert_eq!(nals.len(), 1);
        assert_eq!(nals[0][0], 0x44);
        assert_eq!(nals[0][1], 0x01);
        assert_eq!(nals[0][2], 0x12);
    }

    #[test]
    fn test_poc_computation_idr() {
        let mut decoder = Decoder::with_defaults();
        decoder.active_params.log2_max_pic_order_cnt_lsb = 8;

        // IDR resets POC to 0.
        let poc = decoder.compute_poc(0, true);
        assert_eq!(poc, 0);
    }

    #[test]
    fn test_poc_computation_sequential() {
        let mut decoder = Decoder::with_defaults();
        decoder.active_params.log2_max_pic_order_cnt_lsb = 8;

        // IDR
        let poc0 = decoder.compute_poc(0, true);
        assert_eq!(poc0, 0);

        // Sequential POC LSBs.
        let poc1 = decoder.compute_poc(2, false);
        assert_eq!(poc1, 2);

        let poc2 = decoder.compute_poc(4, false);
        assert_eq!(poc2, 4);

        let poc3 = decoder.compute_poc(6, false);
        assert_eq!(poc3, 6);
    }

    #[test]
    fn test_poc_computation_wraparound() {
        let mut decoder = Decoder::with_defaults();
        decoder.active_params.log2_max_pic_order_cnt_lsb = 4; // max_poc_lsb = 16

        // IDR
        let poc0 = decoder.compute_poc(0, true);
        assert_eq!(poc0, 0);

        // Step through sequential POC LSBs to reach near the wraparound.
        let poc2 = decoder.compute_poc(2, false);
        assert_eq!(poc2, 2);

        let poc6 = decoder.compute_poc(6, false);
        assert_eq!(poc6, 6);

        let poc10 = decoder.compute_poc(10, false);
        assert_eq!(poc10, 10);

        let poc14 = decoder.compute_poc(14, false);
        assert_eq!(poc14, 14);

        // Wraparound: LSB goes from 14 to 2 (delta of -12, which is >= max/2=8)
        // → MSB increases by 16.
        let poc18 = decoder.compute_poc(2, false);
        assert_eq!(poc18, 18); // 16 + 2
    }

    #[test]
    fn test_active_params_from_sps() {
        use crate::nal::{ProfileTierLevel, ScalingListData, Sps};

        let sps = Sps {
            vps_id: 0,
            max_sub_layers_minus1: 0,
            temporal_id_nesting_flag: true,
            profile_tier_level: ProfileTierLevel {
                general_level_idc: 120,
                ..ProfileTierLevel::default()
            },
            sps_id: 0,
            chroma_format_idc: 1,
            separate_colour_plane_flag: false,
            pic_width_in_luma_samples: 1920,
            pic_height_in_luma_samples: 1088,
            conf_win_left_offset: 0,
            conf_win_right_offset: 0,
            conf_win_top_offset: 0,
            conf_win_bottom_offset: 16,
            bit_depth_luma: 8,
            bit_depth_chroma: 8,
            log2_max_pic_order_cnt_lsb: 8,
            max_dec_pic_buffering_minus1: vec![4],
            max_num_reorder_pics: vec![2],
            max_latency_increase_plus1: vec![0],
            log2_min_luma_coding_block_size: 3,
            log2_diff_max_min_luma_coding_block_size: 3,
            log2_min_luma_transform_block_size: 2,
            log2_diff_max_min_luma_transform_block_size: 3,
            max_transform_hierarchy_depth_inter: 1,
            max_transform_hierarchy_depth_intra: 1,
            scaling_list_enabled_flag: false,
            scaling_list_data: ScalingListData::default(),
            amp_enabled_flag: true,
            sample_adaptive_offset_enabled_flag: true,
            pcm_enabled_flag: false,
            pcm_sample_bit_depth_luma_minus1: 0,
            pcm_sample_bit_depth_chroma_minus1: 0,
            log2_min_pcm_luma_coding_block_size_minus3: 0,
            log2_diff_max_min_pcm_luma_coding_block_size: 0,
            pcm_loop_filter_disabled_flag: false,
            short_term_ref_pic_sets: vec![],
            long_term_ref_pics_present_flag: false,
            num_long_term_ref_pics_sps: 0,
            lt_ref_pic_poc_lsb_sps: vec![],
            used_by_curr_pic_lt_sps_flag: vec![],
            temporal_mvp_enabled_flag: true,
            strong_intra_smoothing_enabled_flag: true,
        };

        let params = active_params_from_sps(&sps);
        assert_eq!(params.pic_width, 1920);
        assert_eq!(params.pic_height, 1072); // 1088 - 16 = 1072
        assert_eq!(params.coded_width, 1920);
        assert_eq!(params.coded_height, 1088);
        assert_eq!(params.chroma_format_idc, 1);
        assert_eq!(params.bit_depth_luma, 8);
        assert_eq!(params.level_idc, 120);
        assert_eq!(params.log2_max_pic_order_cnt_lsb, 8);
        assert_eq!(params.max_dec_pic_buffering, 5); // 4 + 1
        assert_eq!(params.max_num_reorder_pics, 2);
        assert_eq!(params.sps_id, 0);
    }

    #[test]
    fn test_convert_output_format_passthrough() {
        let decoder = Decoder::with_defaults();
        let y = vec![128u8; 4];
        let u = vec![128u8; 1];
        let v = vec![128u8; 1];
        let frame = DecodedFrame::from_yuv420p(y, u, v, 2, 2);
        let frames = decoder.convert_output_format(vec![frame]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].pixel_format(), PixelFormat::Yuv420p);
    }

    #[test]
    fn test_convert_output_format_rgb24() {
        let config = DecoderConfig::new().pixel_format(PixelFormat::Rgb24);
        let decoder = Decoder::new(config);
        let y = vec![128u8; 4];
        let u = vec![128u8; 1];
        let v = vec![128u8; 1];
        let frame = DecodedFrame::from_yuv420p(y, u, v, 2, 2);
        let frames = decoder.convert_output_format(vec![frame]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].pixel_format(), PixelFormat::Rgb24);
    }

    #[test]
    fn test_convert_output_format_empty() {
        let decoder = Decoder::with_defaults();
        let frames = decoder.convert_output_format(Vec::new());
        assert!(frames.is_empty());
    }
}

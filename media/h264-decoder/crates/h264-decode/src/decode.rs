//! Top-level decoder API.
//!
//! [`Decoder`] is the main entry point for decoding H.264 Annex B bytestreams
//! into raw video frames.  It wraps the NAL parsing layer (powered by
//! [`h264_reader`]), the decoded picture buffer, and the reconstruction
//! pipeline into a single, easy-to-use interface.
//!
//! # Example
//!
//! ```no_run
//! use h264_decode::{Decoder, DecoderConfig, PixelFormat};
//!
//! let config = DecoderConfig::new().pixel_format(PixelFormat::Yuv420p);
//! let mut decoder = Decoder::new(config);
//!
//! let h264_bytes: &[u8] = &[/* Annex B data */];
//! let frames = decoder.decode(h264_bytes).expect("decode error");
//!
//! for frame in &frames {
//!     println!("{}x{} frame, {} bytes", frame.width(), frame.height(), frame.data().len());
//! }
//!
//! let trailing = decoder.flush().expect("flush error");
//! ```

use h264_reader::nal::sps::{ChromaFormat, FrameMbsFlags, PicOrderCntType, SeqParameterSet};

use crate::dpb::{Dpb, max_dpb_frames};
use crate::error::{DecodeError, DecodeResult, DecodeWarning};
use crate::frame::{DecodedFrame, PictureType};
use crate::nal_handler::{ExpGolombReader, NalAccumulator, NalUnitType};
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
    /// If `true`, apply the in-loop deblocking filter.  Defaults to `true`.
    deblock: bool,
}

impl DecoderConfig {
    /// Create a new default configuration.
    ///
    /// Defaults:
    /// - pixel format: [`PixelFormat::Yuv420p`]
    /// - colour matrix: [`ColourMatrix::Bt601`]
    /// - deblocking: enabled
    pub fn new() -> Self {
        Self {
            pixel_format: PixelFormat::Yuv420p,
            colour_matrix: ColourMatrix::Bt601,
            collect_warnings: false,
            max_ref_frames: None,
            deblock: true,
        }
    }

    /// Set the desired output pixel format.
    ///
    /// When set to a YUV format, the decoder outputs its native planes
    /// directly.  When set to an RGB format, an automatic conversion is
    /// applied using the configured [`ColourMatrix`].
    pub fn pixel_format(mut self, fmt: PixelFormat) -> Self {
        self.pixel_format = fmt;
        self
    }

    /// Set the colour matrix used for YUV→RGB conversion.
    pub fn colour_matrix(mut self, matrix: ColourMatrix) -> Self {
        self.colour_matrix = matrix;
        self
    }

    /// Enable or disable collection of non-fatal warnings.
    pub fn collect_warnings(mut self, enable: bool) -> Self {
        self.collect_warnings = enable;
        self
    }

    /// Override the maximum number of reference frames.
    ///
    /// By default the decoder derives this from the H.264 level in the SPS.
    /// Setting this can be useful for constrained environments.
    pub fn max_ref_frames(mut self, n: usize) -> Self {
        self.max_ref_frames = Some(n);
        self
    }

    /// Enable or disable the in-loop deblocking filter.
    pub fn deblock(mut self, enable: bool) -> Self {
        self.deblock = enable;
        self
    }
}

impl Default for DecoderConfig {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Decoder state
// ---------------------------------------------------------------------------

/// Active sequence/picture parameters derived from the most recently
/// activated SPS/PPS.
#[derive(Debug, Clone)]
struct ActiveParams {
    /// Picture width in luma pixels.
    pic_width: u32,
    /// Picture height in luma pixels.
    pic_height: u32,
    /// Picture width in macroblocks.
    pic_width_in_mbs: u32,
    /// Picture height in macroblock rows.
    pic_height_in_map_units: u32,
    /// Chroma format (1 = 4:2:0, 2 = 4:2:2, 3 = 4:4:4).
    chroma_format_idc: u8,
    /// Luma bit depth.
    bit_depth_luma: u8,
    /// Chroma bit depth.
    bit_depth_chroma: u8,
    /// H.264 level indicator (e.g. 31 for Level 3.1).
    level_idc: u8,
    /// Maximum number of reference frames from the SPS.
    max_num_ref_frames: u32,
    /// Maximum reorder depth from VUI (or derived).
    max_num_reorder_frames: u32,
    /// `log2_max_frame_num_minus4` from SPS.
    log2_max_frame_num: u32,
    /// Picture order count type (0, 1, or 2).
    pic_order_cnt_type: u8,
    /// `log2_max_pic_order_cnt_lsb_minus4` (only for poc type 0).
    log2_max_pic_order_cnt_lsb: u32,
}

impl Default for ActiveParams {
    fn default() -> Self {
        Self {
            pic_width: 0,
            pic_height: 0,
            pic_width_in_mbs: 0,
            pic_height_in_map_units: 0,
            chroma_format_idc: 1,
            bit_depth_luma: 8,
            bit_depth_chroma: 8,
            level_idc: 31,
            max_num_ref_frames: 4,
            max_num_reorder_frames: 4,
            log2_max_frame_num: 4,
            pic_order_cnt_type: 0,
            log2_max_pic_order_cnt_lsb: 4,
        }
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// H.264 (AVC) video decoder.
///
/// Decodes an Annex B bytestream into [`DecodedFrame`]s.  The decoder is
/// designed to be fed data incrementally – each call to [`Decoder::decode`]
/// pushes bytes into the parsing pipeline and returns any frames that are
/// ready for display.  At end-of-stream, call [`Decoder::flush`] to drain
/// buffered frames.
///
/// # Thread safety
///
/// The decoder is `Send` but **not** `Sync`.  It is designed to be used
/// from a single thread (or moved between threads, but not shared).
pub struct Decoder {
    /// User-supplied configuration.
    config: DecoderConfig,
    /// NAL unit accumulator / parser bridge.
    nal_acc: NalAccumulator,
    /// Decoded picture buffer for reference management and reordering.
    dpb: Dpb,
    /// Currently active sequence/picture parameters.
    active_params: ActiveParams,
    /// Monotonically increasing frame counter (decode order).
    decode_frame_counter: u64,
    /// Current picture order count state.
    poc_state: PocState,
    /// Accumulated non-fatal warnings (if collection is enabled).
    warnings: Vec<DecodeWarning>,
    /// Annex B start-code scanner state for incremental feeding.
    annex_b_buffer: Vec<u8>,
}

/// Internal POC (picture order count) tracking state.
#[derive(Debug, Clone, Default)]
struct PocState {
    prev_pic_order_cnt_msb: i32,
    prev_pic_order_cnt_lsb: u32,
    pic_order_cnt_msb: i32,
}

impl Decoder {
    /// Create a new decoder with the given configuration.
    pub fn new(config: DecoderConfig) -> Self {
        Self {
            config,
            nal_acc: NalAccumulator::new(),
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

    /// Feed raw H.264 Annex B bytestream data into the decoder.
    ///
    /// The input `data` does not need to be aligned to NAL unit boundaries –
    /// the decoder maintains an internal buffer and handles incremental
    /// parsing.
    ///
    /// Returns a (possibly empty) list of decoded frames that are ready for
    /// display, in display order (sorted by picture order count).
    pub fn decode(&mut self, data: &[u8]) -> DecodeResult<Vec<DecodedFrame>> {
        // Append incoming data to the Annex B buffer and split on start
        // codes to extract complete NAL units.
        self.annex_b_buffer.extend_from_slice(data);
        let nal_units = self.extract_nal_units();

        let mut output_frames = Vec::new();

        for nal_data in nal_units {
            if nal_data.is_empty() {
                continue;
            }

            // First byte is the NAL header.
            let nal_header_byte = nal_data[0];
            let nal_ref_idc = (nal_header_byte >> 5) & 0x03;
            let nal_unit_type_raw = nal_header_byte & 0x1F;
            let nal_type = NalUnitType::from_raw(nal_unit_type_raw);

            // RBSP payload is everything after the header byte, with
            // emulation prevention bytes removed.
            let rbsp = remove_emulation_prevention(&nal_data[1..]);

            match nal_type {
                NalUnitType::Sps => {
                    self.handle_sps(&rbsp)?;
                }
                NalUnitType::Pps => {
                    self.handle_pps(&rbsp)?;
                }
                NalUnitType::SliceIdr | NalUnitType::SliceNonIdr => {
                    let frames = self.handle_slice(nal_type, nal_ref_idc, &rbsp)?;
                    output_frames.extend(frames);
                }
                NalUnitType::AccessUnitDelimiter => {
                    log::trace!("access unit delimiter");
                }
                NalUnitType::Sei => {
                    log::trace!("SEI NAL unit ({} bytes)", rbsp.len());
                }
                NalUnitType::EndOfSequence => {
                    let flushed = self.dpb.flush();
                    output_frames.extend(self.convert_output_format(flushed));
                }
                NalUnitType::EndOfStream => {
                    let flushed = self.dpb.flush();
                    output_frames.extend(self.convert_output_format(flushed));
                }
                _ => {
                    if self.config.collect_warnings {
                        self.warnings.push(DecodeWarning::SkippedNalUnit {
                            nal_unit_type: nal_unit_type_raw,
                        });
                    }
                    log::debug!("skipping NAL unit type {nal_unit_type_raw}");
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
        self.nal_acc.reset();
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

        // Find all start-code positions.
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
            // Keep the entire buffer.
            return nals;
        }

        // Extract NAL units between consecutive start codes.
        for window in positions.windows(2) {
            let start = window[0];
            let end = window[1];

            // `end` points to the byte after the start code, so we need to
            // find where the start code of the *next* NAL begins.
            // `end` = positions[i+1] = byte after the start code prefix.
            // The start code is either 3 or 4 bytes.  We want
            // nal_data_end = sc_start, where buf[sc_start..end] is either
            // [00, 00, 01] or [00, 00, 00, 01].
            //
            // Try 4-byte variant first.
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
        // Find the start of this last start code so we keep the prefix too.
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

    /// Handle a parsed SPS NAL unit using h264-reader's [`SeqParameterSet`]
    /// parser for correct, spec-compliant extraction of all SPS fields.
    fn handle_sps(&mut self, rbsp: &[u8]) -> DecodeResult<()> {
        // Delegate to h264-reader via our NalAccumulator, which calls
        // SeqParameterSet::from_bits() and stores the result.
        let sps_id = self.nal_acc.parse_sps(rbsp)?;

        let sps = self
            .nal_acc
            .sps_store
            .get(sps_id)
            .ok_or(DecodeError::MissingSps(sps_id as u8))?;

        // Extract the fields we need into our ActiveParams.
        let params = active_params_from_sps(sps);

        log::info!(
            "SPS id={sps_id}: profile={:?}, level={}, {}x{} \
             ({} x {} MBs), chroma={}, ref_frames={}",
            sps.profile_idc,
            params.level_idc,
            params.pic_width,
            params.pic_height,
            params.pic_width_in_mbs,
            params.pic_height_in_map_units,
            params.chroma_format_idc,
            params.max_num_ref_frames,
        );

        // Resize DPB to match the level / picture size.
        let dpb_size = self.config.max_ref_frames.unwrap_or_else(|| {
            max_dpb_frames(
                params.level_idc,
                params.pic_width_in_mbs,
                params.pic_height_in_map_units,
            )
        });
        let bumped = self.dpb.set_max_size(dpb_size);
        if !bumped.is_empty() {
            log::debug!("SPS change bumped {} frames from DPB", bumped.len());
        }
        self.dpb
            .set_max_num_reorder_frames(params.max_num_ref_frames as usize);

        self.active_params = params;

        Ok(())
    }

    /// Handle a parsed PPS NAL unit using h264-reader's [`PicParameterSet`]
    /// parser.
    fn handle_pps(&mut self, rbsp: &[u8]) -> DecodeResult<()> {
        let pps_id = self.nal_acc.parse_pps(rbsp)?;
        log::debug!("PPS id={pps_id} parsed and stored");
        Ok(())
    }

    /// Handle a coded slice NAL unit – this is where actual frame decoding
    /// happens.
    fn handle_slice(
        &mut self,
        nal_type: NalUnitType,
        nal_ref_idc: u8,
        rbsp: &[u8],
    ) -> DecodeResult<Vec<DecodedFrame>> {
        if rbsp.is_empty() {
            return Err(DecodeError::InvalidBitstream("empty slice NAL unit".into()));
        }

        let is_idr = nal_type.is_idr();
        let is_reference = nal_ref_idc > 0;

        // Parse minimal slice header fields.
        let mut reader = ExpGolombReader::new(rbsp);
        let first_mb_in_slice = reader.read_ue().unwrap_or(0);
        let slice_type_raw = reader.read_ue().unwrap_or(2);
        let _pps_id = reader.read_ue().unwrap_or(0);

        let picture_type = match slice_type_raw % 5 {
            0 => PictureType::P,
            1 => PictureType::B,
            2 => PictureType::I,
            3 => PictureType::P, // SP
            4 => PictureType::I, // SI
            _ => PictureType::I,
        };

        // Read frame_num.
        let frame_num_bits = self.active_params.log2_max_frame_num as u8;
        let frame_num = reader.read_bits(frame_num_bits).unwrap_or(0);

        // Compute picture order count.
        let poc = self.compute_poc(frame_num, is_idr, &mut reader);

        let params = &self.active_params;
        let pic_width = params.pic_width;
        let pic_height = params.pic_height;

        if pic_width == 0 || pic_height == 0 {
            return Err(DecodeError::InvalidBitstream(
                "slice received before SPS (no picture dimensions)".into(),
            ));
        }

        log::debug!(
            "slice: type={picture_type}, frame_num={frame_num}, poc={poc}, \
             idr={is_idr}, ref={is_reference}, first_mb={first_mb_in_slice}, \
             {pic_width}x{pic_height}"
        );

        // ----- Decode the picture -----
        //
        // In a complete H.264 decoder this is where we would:
        //   1. Entropy-decode the macroblock layer (CAVLC or CABAC).
        //   2. Perform intra prediction or motion-compensated inter
        //      prediction for each macroblock / sub-macroblock.
        //   3. Inverse-transform and dequantize the residual.
        //   4. Add the residual to the prediction to reconstruct the
        //      picture.
        //   5. Apply the in-loop deblocking filter.
        //
        // The infrastructure for these steps exists in sibling modules
        // (`transform`, `dpb`, `nal_handler`).  The reconstruction loop
        // will be filled in as the decoder is fleshed out.
        //
        // For now we produce a placeholder frame (grey for I-frames,
        // based on the reference for P/B-frames) so that the API contract
        // is fulfilled and downstream consumers can exercise the pipeline.

        let frame = self.reconstruct_picture(
            pic_width,
            pic_height,
            frame_num,
            poc,
            picture_type,
            is_idr,
            is_reference,
        )?;

        self.decode_frame_counter += 1;

        // Insert into DPB (handles reference marking + reorder bumping).
        let dpb_output = self.dpb.insert(frame)?;

        // Apply sliding-window reference marking if needed.
        let max_refs = self
            .config
            .max_ref_frames
            .unwrap_or(self.active_params.max_num_ref_frames as usize);
        self.dpb.sliding_window_mark(max_refs);

        Ok(self.convert_output_format(dpb_output))
    }

    // ------------------------------------------------------------------
    // Picture order count computation
    // ------------------------------------------------------------------

    fn compute_poc(
        &mut self,
        frame_num: u32,
        is_idr: bool,
        reader: &mut ExpGolombReader<'_>,
    ) -> i32 {
        match self.active_params.pic_order_cnt_type {
            0 => {
                let max_poc_lsb = 1u32 << self.active_params.log2_max_pic_order_cnt_lsb;
                let poc_lsb = reader
                    .read_bits(self.active_params.log2_max_pic_order_cnt_lsb as u8)
                    .unwrap_or(0);

                if is_idr {
                    self.poc_state.prev_pic_order_cnt_msb = 0;
                    self.poc_state.prev_pic_order_cnt_lsb = 0;
                }

                let prev_msb = self.poc_state.prev_pic_order_cnt_msb;
                let prev_lsb = self.poc_state.prev_pic_order_cnt_lsb;

                let msb = if poc_lsb < prev_lsb
                    && (prev_lsb.wrapping_sub(poc_lsb)) >= max_poc_lsb / 2
                {
                    prev_msb + max_poc_lsb as i32
                } else if poc_lsb > prev_lsb && (poc_lsb.wrapping_sub(prev_lsb)) > max_poc_lsb / 2 {
                    prev_msb - max_poc_lsb as i32
                } else {
                    prev_msb
                };

                self.poc_state.prev_pic_order_cnt_msb = msb;
                self.poc_state.prev_pic_order_cnt_lsb = poc_lsb;
                self.poc_state.pic_order_cnt_msb = msb;

                msb + poc_lsb as i32
            }
            2 => {
                // POC type 2: POC = 2 * frame_num (simplified).
                if is_idr { 0 } else { (2 * frame_num) as i32 }
            }
            _ => {
                // POC type 1 or unknown – fall back to frame_num.
                frame_num as i32
            }
        }
    }

    // ------------------------------------------------------------------
    // Picture reconstruction
    // ------------------------------------------------------------------

    /// Reconstruct a decoded picture.
    ///
    /// In the current scaffold this produces a YUV 4:2:0 frame.  For
    /// I-frames a mid-grey plane is generated; for P/B-frames the first
    /// available reference is copied (or grey if no reference exists).
    #[allow(clippy::too_many_arguments)]
    fn reconstruct_picture(
        &self,
        width: u32,
        height: u32,
        frame_num: u32,
        poc: i32,
        picture_type: PictureType,
        is_idr: bool,
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
                let ref_list = self.dpb.build_ref_list_0(frame_num);
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
            .with_frame_num(frame_num)
            .with_pic_order_cnt(poc)
            .with_is_reference(is_reference)
            .with_is_idr(is_idr)
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
        Self::with_defaults()
    }
}

// ---------------------------------------------------------------------------
// Annex B emulation prevention removal
// ---------------------------------------------------------------------------

/// Remove emulation prevention bytes (`0x03`) from an RBSP.
///
/// In H.264 Annex B, the byte sequence `00 00 03` is used to prevent
/// accidental start-code patterns inside NAL unit data.  The `03` byte
/// must be stripped to recover the true RBSP.
fn remove_emulation_prevention(data: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if i + 2 < data.len() && data[i] == 0x00 && data[i + 1] == 0x00 && data[i + 2] == 0x03 {
            rbsp.push(0x00);
            rbsp.push(0x00);
            i += 3; // skip the 0x03 byte
        } else {
            rbsp.push(data[i]);
            i += 1;
        }
    }
    rbsp
}

// ---------------------------------------------------------------------------
// Extract ActiveParams from an h264-reader SeqParameterSet
// ---------------------------------------------------------------------------

/// Build [`ActiveParams`] from an h264-reader [`SeqParameterSet`].
fn active_params_from_sps(sps: &SeqParameterSet) -> ActiveParams {
    let pic_width_in_mbs = sps.pic_width_in_mbs();
    let pic_height_in_map_units = sps.pic_height_in_map_units();

    // pixel_dimensions() accounts for cropping; fall back to raw MB math.
    let (pic_width, pic_height) = sps.pixel_dimensions().unwrap_or_else(|_| {
        let mbs_height_factor = match sps.frame_mbs_flags {
            FrameMbsFlags::Frames => 1u32,
            FrameMbsFlags::Fields { .. } => 2,
        };
        (
            pic_width_in_mbs * 16,
            pic_height_in_map_units * mbs_height_factor * 16,
        )
    });

    let chroma_format_idc = match sps.chroma_info.chroma_format {
        ChromaFormat::Monochrome => 0u8,
        ChromaFormat::YUV420 => 1,
        ChromaFormat::YUV422 => 2,
        ChromaFormat::YUV444 => 3,
        ChromaFormat::Invalid(v) => v as u8,
    };

    let bit_depth_luma = sps.chroma_info.bit_depth_luma_minus8 + 8;
    let bit_depth_chroma = sps.chroma_info.bit_depth_chroma_minus8 + 8;

    let level_idc = sps.level_idc;
    let max_num_ref_frames = sps.max_num_ref_frames;

    let log2_max_frame_num = sps.log2_max_frame_num() as u32;

    let (pic_order_cnt_type, log2_max_pic_order_cnt_lsb) = match &sps.pic_order_cnt {
        PicOrderCntType::TypeZero {
            log2_max_pic_order_cnt_lsb_minus4,
        } => (0u8, (*log2_max_pic_order_cnt_lsb_minus4 as u32) + 4),
        PicOrderCntType::TypeOne { .. } => (1, 0),
        PicOrderCntType::TypeTwo => (2, 0),
    };

    let max_num_reorder_frames = sps
        .vui_parameters
        .as_ref()
        .and_then(|vui| vui.bitstream_restrictions.as_ref())
        .map(|br| br.max_num_reorder_frames)
        .unwrap_or(max_num_ref_frames);

    ActiveParams {
        pic_width,
        pic_height,
        pic_width_in_mbs,
        pic_height_in_map_units,
        chroma_format_idc,
        bit_depth_luma,
        bit_depth_chroma,
        level_idc,
        max_num_ref_frames,
        max_num_reorder_frames,
        log2_max_frame_num,
        pic_order_cnt_type,
        log2_max_pic_order_cnt_lsb,
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
        assert_eq!(config.colour_matrix, ColourMatrix::Bt601);
        assert!(config.deblock);
        assert!(!config.collect_warnings);
        assert!(config.max_ref_frames.is_none());
    }

    #[test]
    fn test_decoder_config_builder() {
        let config = DecoderConfig::new()
            .pixel_format(PixelFormat::Rgb24)
            .colour_matrix(ColourMatrix::Bt709)
            .deblock(false)
            .collect_warnings(true)
            .max_ref_frames(2);

        assert_eq!(config.pixel_format, PixelFormat::Rgb24);
        assert_eq!(config.colour_matrix, ColourMatrix::Bt709);
        assert!(!config.deblock);
        assert!(config.collect_warnings);
        assert_eq!(config.max_ref_frames, Some(2));
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
        decoder.active_params.pic_width = 1920;
        decoder.decode_frame_counter = 42;
        decoder.reset();
        assert_eq!(decoder.width(), 0);
        assert_eq!(decoder.decoded_frame_count(), 0);
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
        let mut decoder = Decoder::with_defaults();
        // Random bytes with no start codes should produce nothing.
        let frames = decoder.decode(&[0x12, 0x34, 0x56, 0x78]).unwrap();
        assert!(frames.is_empty());
    }

    #[test]
    fn test_remove_emulation_prevention_none() {
        let data = [0x01, 0x02, 0x03, 0x04];
        let rbsp = remove_emulation_prevention(&data);
        assert_eq!(rbsp, data);
    }

    #[test]
    fn test_remove_emulation_prevention_basic() {
        // 00 00 03 should become 00 00
        let data = [0x00, 0x00, 0x03, 0x01];
        let rbsp = remove_emulation_prevention(&data);
        assert_eq!(rbsp, [0x00, 0x00, 0x01]);
    }

    #[test]
    fn test_remove_emulation_prevention_multiple() {
        let data = [0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0xFF];
        let rbsp = remove_emulation_prevention(&data);
        assert_eq!(rbsp, [0x00, 0x00, 0x00, 0x00, 0xFF]);
    }

    #[test]
    fn test_remove_emulation_prevention_at_end() {
        let data = [0x00, 0x00, 0x03];
        let rbsp = remove_emulation_prevention(&data);
        assert_eq!(rbsp, [0x00, 0x00]);
    }

    #[test]
    fn test_exp_golomb_reader_ue() {
        // ue(0) = '1'
        let data = [0x80];
        let mut r = ExpGolombReader::new(&data);
        assert_eq!(r.read_ue(), Some(0));

        // ue(1) = '010'
        let data = [0x40];
        let mut r = ExpGolombReader::new(&data);
        assert_eq!(r.read_ue(), Some(1));

        // ue(5) = '00110'
        let data = [0x30]; // 0b00110000
        let mut r = ExpGolombReader::new(&data);
        assert_eq!(r.read_ue(), Some(5));
    }

    #[test]
    fn test_exp_golomb_reader_se() {
        // se: ue(0)->0, ue(1)->1, ue(2)->-1, ue(3)->2, ue(4)->-2
        let data = [0x60]; // ue(2)
        let mut r = ExpGolombReader::new(&data);
        assert_eq!(r.read_se(), Some(-1));
    }

    #[test]
    fn test_annex_b_extract_single_nal() {
        let mut decoder = Decoder::with_defaults();
        // Two NAL units with 3-byte start codes.
        // NAL 1: [0x67, 0x42, 0x00, 0x1E] (SPS-like)
        // NAL 2: [0x68, 0xCE, 0x38, 0x80] (PPS-like)
        let mut data = Vec::new();
        data.extend_from_slice(&[0x00, 0x00, 0x01]); // start code 1
        data.extend_from_slice(&[0x67, 0x42, 0x00, 0x1E]);
        data.extend_from_slice(&[0x00, 0x00, 0x01]); // start code 2
        data.extend_from_slice(&[0x68, 0xCE, 0x38, 0x80]);
        data.extend_from_slice(&[0x00, 0x00, 0x01]); // trailing start code
        data.extend_from_slice(&[0x65]); // beginning of next NAL

        decoder.annex_b_buffer = data;
        let nals = decoder.extract_nal_units();
        // Should extract at least 2 complete NAL units.
        assert!(nals.len() >= 2, "got {} NALs", nals.len());
        assert_eq!(nals[0][0], 0x67); // SPS header byte
        assert_eq!(nals[1][0], 0x68); // PPS header byte
    }

    #[test]
    fn test_decoder_with_minimal_sps() {
        let mut decoder = Decoder::with_defaults();

        // Craft a minimal Baseline-profile SPS RBSP for a 16×16 picture (1 MB).
        // The bytes are hand-constructed to match what h264-reader expects
        // from SeqParameterSet::from_bits():
        //
        //   profile_idc      = 66  (Baseline)                  u(8)  = 0x42
        //   constraint_set0  = 1 (set)                         u(1)
        //   constraint_set1  = 0                               u(1)
        //   constraint_set2  = 0                               u(1)
        //   constraint_set3  = 0                               u(1)
        //   constraint_set4  = 0                               u(1)
        //   constraint_set5  = 0                               u(1)
        //   reserved_zero_2  = 00                              u(2)  => 0x80
        //   level_idc        = 10                              u(8)  = 0x0A
        //   sps_id           = 0         ue => '1'
        //   log2_max_fn-4    = 0         ue => '1'
        //   poc_type         = 2         ue => '011'
        //   max_ref_frames   = 1         ue => '010'
        //   gaps_allowed     = 0         u(1) => '0'
        //   w_mbs-1          = 0         ue => '1'
        //   h_map_units-1    = 0         ue => '1'
        //   frame_mbs_only   = 1         u(1) => '1'
        //   direct_8x8_inf   = 0         u(1) => '0'
        //   frame_cropping   = 0         u(1) => '0'
        //   vui_present      = 0         u(1) => '0'
        //
        // Bits after the 3 fixed bytes:
        //   1 1 011 010 0 1 1 1 0 0 0 1 00 = 11011010 01110001
        //   = 0xDA 0x71
        //
        // The final '1' is the RBSP stop bit, followed by zero-padding to
        // the byte boundary.  An extra 0x00 byte is appended so that
        // h264-reader's `finish()` does not hit an unexpected EOF when
        // verifying trailing bits.
        let rbsp = vec![0x42, 0x80, 0x0A, 0xDA, 0x71, 0x00];
        let result = decoder.handle_sps(&rbsp);
        assert!(result.is_ok(), "SPS parse failed: {:?}", result.err());
        assert_eq!(decoder.width(), 16);
        assert_eq!(decoder.height(), 16);
    }

    #[test]
    fn test_decoder_warnings_collection() {
        let config = DecoderConfig::new().collect_warnings(true);
        let mut decoder = Decoder::new(config);
        decoder
            .warnings
            .push(DecodeWarning::SkippedNalUnit { nal_unit_type: 12 });
        let warnings = decoder.take_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(decoder.take_warnings().is_empty());
    }

    #[test]
    fn test_convert_output_format_yuv420p_passthrough() {
        let decoder = Decoder::with_defaults();
        let frame = DecodedFrame::from_yuv420p(vec![128; 16], vec![128; 4], vec![128; 4], 4, 4);
        let converted = decoder.convert_output_format(vec![frame]);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].pixel_format(), PixelFormat::Yuv420p);
    }

    #[test]
    fn test_convert_output_format_rgb24() {
        let config = DecoderConfig::new().pixel_format(PixelFormat::Rgb24);
        let decoder = Decoder::new(config);
        let frame = DecodedFrame::from_yuv420p(vec![128; 16], vec![128; 4], vec![128; 4], 4, 4);
        let converted = decoder.convert_output_format(vec![frame]);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].pixel_format(), PixelFormat::Rgb24);
    }

    #[test]
    fn test_convert_output_format_empty() {
        let decoder = Decoder::with_defaults();
        let converted = decoder.convert_output_format(vec![]);
        assert!(converted.is_empty());
    }
}

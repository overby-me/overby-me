//! Decoded Picture Buffer (DPB) for the H.265/HEVC decoder.
//!
//! The DPB stores decoded pictures that are either needed for reference by
//! future pictures or waiting to be output in display order (by picture
//! order count).
//!
//! HEVC DPB management differs from H.264 in several ways:
//!
//! * Reference pictures are managed via Reference Picture Sets (RPS) rather
//!   than sliding-window / MMCO commands.
//! * The maximum DPB size is derived from the SPS level and picture size
//!   (`sps_max_dec_pic_buffering_minus1`).
//! * IRAP pictures (IDR, CRA, BLA) trigger specific DPB flush / output
//!   behaviour.
//!
//! # Bumping process
//!
//! When the DPB is full and a new picture needs to be inserted, the picture
//! with the smallest POC that is marked for output is "bumped" (emitted to
//! the output queue and its output-needed flag cleared).  If the bumped
//! picture is also not used for reference, it is removed from the DPB
//! entirely.

use crate::error::{DecodeError, DecodeResult};
use crate::frame::{DecodedFrame, PictureType};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Absolute upper bound on DPB size to prevent unbounded memory usage.
const ABSOLUTE_MAX_DPB_FRAMES: usize = 16;

/// Default DPB capacity when no SPS has been parsed yet.
const DEFAULT_DPB_SIZE: usize = 6;

// ---------------------------------------------------------------------------
// DPB entry
// ---------------------------------------------------------------------------

/// Reference status of a picture in the DPB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReferenceStatus {
    /// Not used for reference (may be evicted once output is done).
    Unused,
    /// Used for short-term reference.
    ShortTerm,
    /// Used for long-term reference.
    LongTerm,
}

/// A single entry in the decoded picture buffer.
#[derive(Debug, Clone)]
struct DpbEntry {
    /// The decoded frame data.
    frame: DecodedFrame,
    /// Current reference status.
    reference: ReferenceStatus,
    /// Whether this picture still needs to be output (displayed).
    needed_for_output: bool,
    /// Picture order count (display order).
    pic_order_cnt: i32,
    /// Decode order counter (monotonically increasing).
    decode_order: u64,
}

impl DpbEntry {
    fn new(frame: DecodedFrame, decode_order: u64) -> Self {
        let poc = frame.pic_order_cnt();
        let is_ref = frame.is_reference();
        Self {
            frame,
            reference: if is_ref {
                ReferenceStatus::ShortTerm
            } else {
                ReferenceStatus::Unused
            },
            needed_for_output: true,
            pic_order_cnt: poc,
            decode_order,
        }
    }

    /// A picture is evictable when it is neither needed for output nor
    /// used as a reference.
    fn is_evictable(&self) -> bool {
        !self.needed_for_output && self.reference == ReferenceStatus::Unused
    }

    fn is_reference(&self) -> bool {
        self.reference != ReferenceStatus::Unused
    }

    fn is_short_term_ref(&self) -> bool {
        self.reference == ReferenceStatus::ShortTerm
    }

    fn is_long_term_ref(&self) -> bool {
        self.reference == ReferenceStatus::LongTerm
    }
}

// ---------------------------------------------------------------------------
// DPB
// ---------------------------------------------------------------------------

/// The decoded picture buffer.
///
/// Manages storage, reference marking, reorder bumping, and output of
/// decoded HEVC pictures.
///
/// # Output ordering
///
/// Pictures are emitted in display order (ascending POC).  The DPB
/// internally buffers up to `max_num_reorder_pics` pictures before
/// releasing them.
pub struct Dpb {
    /// Stored pictures.
    entries: Vec<DpbEntry>,
    /// Maximum number of pictures the DPB can hold (derived from SPS).
    max_size: usize,
    /// Maximum number of pictures that may be reordered before output.
    /// Derived from `sps_max_num_reorder_pics`.
    max_num_reorder_pics: usize,
    /// Monotonically increasing decode-order counter.
    decode_counter: u64,
}

impl Dpb {
    /// Create a new, empty DPB with the default capacity.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            max_size: DEFAULT_DPB_SIZE,
            max_num_reorder_pics: 0,
            decode_counter: 0,
        }
    }

    /// Create a DPB with a specific maximum size.
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            entries: Vec::new(),
            max_size: max_size.min(ABSOLUTE_MAX_DPB_FRAMES),
            max_num_reorder_pics: 0,
            decode_counter: 0,
        }
    }

    /// Resize the DPB.  If the new size is smaller than the current
    /// number of entries, excess pictures are bumped for output.
    ///
    /// Returns any pictures bumped as a result.
    pub fn set_max_size(&mut self, max_size: usize) -> Vec<DecodedFrame> {
        self.max_size = max_size.min(ABSOLUTE_MAX_DPB_FRAMES);
        self.bump_until_within_capacity()
    }

    /// Set the maximum number of reorder frames.
    pub fn set_max_num_reorder_pics(&mut self, n: usize) {
        self.max_num_reorder_pics = n;
    }

    /// Current maximum DPB capacity.
    #[inline]
    pub fn max_size(&self) -> usize {
        self.max_size
    }

    /// Number of pictures currently stored.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the DPB is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether the DPB is at capacity.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.entries.len() >= self.max_size
    }

    /// Number of pictures currently marked as short-term reference.
    pub fn num_short_term_refs(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.is_short_term_ref())
            .count()
    }

    /// Number of pictures currently marked as long-term reference.
    pub fn num_long_term_refs(&self) -> usize {
        self.entries.iter().filter(|e| e.is_long_term_ref()).count()
    }

    /// Total reference pictures.
    pub fn num_refs(&self) -> usize {
        self.entries.iter().filter(|e| e.is_reference()).count()
    }

    /// Number of pictures that still need to be output.
    pub fn num_needs_output(&self) -> usize {
        self.entries.iter().filter(|e| e.needed_for_output).count()
    }

    // ------------------------------------------------------------------
    // Insertion
    // ------------------------------------------------------------------

    /// Insert a newly decoded picture into the DPB.
    ///
    /// If the picture is an IDR, the DPB is flushed first (all stored
    /// pictures are output in POC order).
    ///
    /// After insertion, any pictures that should be bumped for reorder
    /// compliance are returned.
    pub fn insert(&mut self, frame: DecodedFrame) -> DecodeResult<Vec<DecodedFrame>> {
        let is_irap = frame.is_irap();
        let is_idr = frame.picture_type() == PictureType::I && is_irap;

        let mut output = Vec::new();

        // IDR pictures flush the entire DPB.
        if is_idr {
            output.extend(self.drain_all_for_output());
        }

        // Evict unreferenced, already-output pictures.
        self.evict_unused();

        // If still full, bump the lowest-POC output-pending picture.
        while self.entries.len() >= self.max_size {
            self.evict_unused();
            if self.entries.len() < self.max_size {
                break;
            }

            if let Some(bumped) = self.bump_one() {
                output.push(bumped);
                self.evict_unused();
            } else {
                // All remaining entries are references that have already
                // been output.  Forcibly evict the oldest short-term ref
                // to make room (this matches decoder behaviour when the
                // DPB is genuinely full of active references).
                if !self.force_evict_oldest_ref() {
                    return Err(DecodeError::DpbOverflow {
                        capacity: self.max_size,
                        frame_num: frame.frame_num(),
                    });
                }
            }
        }

        let entry = DpbEntry::new(frame, self.decode_counter);
        self.decode_counter += 1;
        self.entries.push(entry);

        // Reorder bumping: if we have more output-pending pictures than
        // max_num_reorder_pics, bump the lowest.
        output.extend(self.bump_for_reorder());

        Ok(output)
    }

    // ------------------------------------------------------------------
    // Flush / clear
    // ------------------------------------------------------------------

    /// Flush all output-pending pictures from the DPB, in POC order.
    /// Non-reference pictures are removed; reference pictures have their
    /// output flag cleared.
    pub fn flush(&mut self) -> Vec<DecodedFrame> {
        self.drain_all_for_output()
    }

    /// Completely clear the DPB, discarding all pictures (no output).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    // ------------------------------------------------------------------
    // Reference picture set operations
    // ------------------------------------------------------------------

    /// Mark a picture as "unused for reference" by its POC.
    ///
    /// This is used during RPS (Reference Picture Set) application: any
    /// picture not present in the current RPS is marked unused.
    pub fn mark_unused_by_poc(&mut self, poc: i32) {
        for entry in &mut self.entries {
            if entry.pic_order_cnt == poc && entry.is_reference() {
                entry.reference = ReferenceStatus::Unused;
            }
        }
    }

    /// Mark a picture as short-term reference by its POC.
    pub fn mark_short_term_ref_by_poc(&mut self, poc: i32) {
        for entry in &mut self.entries {
            if entry.pic_order_cnt == poc {
                entry.reference = ReferenceStatus::ShortTerm;
            }
        }
    }

    /// Mark a picture as long-term reference by its POC.
    pub fn mark_long_term_ref_by_poc(&mut self, poc: i32) {
        for entry in &mut self.entries {
            if entry.pic_order_cnt == poc {
                entry.reference = ReferenceStatus::LongTerm;
            }
        }
    }

    /// Mark all pictures as "unused for reference".
    pub fn mark_all_unused(&mut self) {
        for entry in &mut self.entries {
            entry.reference = ReferenceStatus::Unused;
        }
    }

    // ------------------------------------------------------------------
    // Reference picture lookups
    // ------------------------------------------------------------------

    /// Find a short-term reference picture by its POC.
    pub fn find_short_term_ref(&self, poc: i32) -> Option<&DecodedFrame> {
        self.entries
            .iter()
            .find(|e| e.is_short_term_ref() && e.pic_order_cnt == poc)
            .map(|e| &e.frame)
    }

    /// Find a long-term reference picture by its POC.
    pub fn find_long_term_ref(&self, poc: i32) -> Option<&DecodedFrame> {
        self.entries
            .iter()
            .find(|e| e.is_long_term_ref() && e.pic_order_cnt == poc)
            .map(|e| &e.frame)
    }

    /// Find any reference picture by POC.
    pub fn find_ref_by_poc(&self, poc: i32) -> Option<&DecodedFrame> {
        self.entries
            .iter()
            .find(|e| e.is_reference() && e.pic_order_cnt == poc)
            .map(|e| &e.frame)
    }

    /// Build reference picture list 0 (for P and B slices).
    ///
    /// Returns short-term references sorted by POC descending relative to
    /// `current_poc` (closest first), followed by long-term references.
    pub fn build_ref_list_0(&self, current_poc: i32) -> Vec<&DecodedFrame> {
        // Short-term references with POC < current_poc, sorted descending.
        let mut st_before: Vec<&DpbEntry> = self
            .entries
            .iter()
            .filter(|e| e.is_short_term_ref() && e.pic_order_cnt < current_poc)
            .collect();
        st_before.sort_by(|a, b| b.pic_order_cnt.cmp(&a.pic_order_cnt));

        // Short-term references with POC > current_poc, sorted ascending.
        let mut st_after: Vec<&DpbEntry> = self
            .entries
            .iter()
            .filter(|e| e.is_short_term_ref() && e.pic_order_cnt > current_poc)
            .collect();
        st_after.sort_by(|a, b| a.pic_order_cnt.cmp(&b.pic_order_cnt));

        // Long-term references.
        let lt: Vec<&DpbEntry> = self
            .entries
            .iter()
            .filter(|e| e.is_long_term_ref())
            .collect();

        let mut list: Vec<&DecodedFrame> = Vec::new();
        for e in &st_before {
            list.push(&e.frame);
        }
        for e in &st_after {
            list.push(&e.frame);
        }
        for e in &lt {
            list.push(&e.frame);
        }
        list
    }

    /// Build reference picture list 1 (for B slices).
    ///
    /// Returns short-term references sorted by POC ascending relative to
    /// `current_poc` (closest after first), followed by those before,
    /// then long-term references.
    pub fn build_ref_list_1(&self, current_poc: i32) -> Vec<&DecodedFrame> {
        // Short-term references with POC > current_poc, sorted ascending.
        let mut st_after: Vec<&DpbEntry> = self
            .entries
            .iter()
            .filter(|e| e.is_short_term_ref() && e.pic_order_cnt > current_poc)
            .collect();
        st_after.sort_by(|a, b| a.pic_order_cnt.cmp(&b.pic_order_cnt));

        // Short-term references with POC < current_poc, sorted descending.
        let mut st_before: Vec<&DpbEntry> = self
            .entries
            .iter()
            .filter(|e| e.is_short_term_ref() && e.pic_order_cnt < current_poc)
            .collect();
        st_before.sort_by(|a, b| b.pic_order_cnt.cmp(&a.pic_order_cnt));

        // Long-term references.
        let lt: Vec<&DpbEntry> = self
            .entries
            .iter()
            .filter(|e| e.is_long_term_ref())
            .collect();

        let mut list: Vec<&DecodedFrame> = Vec::new();
        for e in &st_after {
            list.push(&e.frame);
        }
        for e in &st_before {
            list.push(&e.frame);
        }
        for e in &lt {
            list.push(&e.frame);
        }
        list
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Bump (output) the picture with the smallest POC that is marked
    /// "needed for output".  If that picture is also not a reference, it
    /// is removed; otherwise only its output flag is cleared.
    ///
    /// Returns `Some(frame)` if a picture was bumped, `None` if no picture
    /// is pending output.
    fn bump_one(&mut self) -> Option<DecodedFrame> {
        // Find the index of the entry with the smallest POC that needs output.
        let idx = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.needed_for_output)
            .min_by_key(|(_, e)| e.pic_order_cnt)
            .map(|(i, _)| i)?;

        let entry = &mut self.entries[idx];
        entry.needed_for_output = false;
        let frame = entry.frame.clone();

        // If also not a reference, remove entirely.
        if entry.is_evictable() {
            self.entries.remove(idx);
        }

        Some(frame)
    }

    /// Bump pictures until the number of entries is within `max_size`.
    fn bump_until_within_capacity(&mut self) -> Vec<DecodedFrame> {
        let mut output = Vec::new();
        while self.entries.len() > self.max_size {
            self.evict_unused();
            if self.entries.len() <= self.max_size {
                break;
            }

            if let Some(f) = self.bump_one() {
                output.push(f);
                self.evict_unused();
            } else {
                // All remaining entries are already-output references.
                // Forcibly evict the oldest to make room.
                if !self.force_evict_oldest_ref() {
                    break;
                }
            }
        }
        output
    }

    /// Bump pictures to satisfy the reorder constraint: the number of
    /// output-pending pictures must not exceed `max_num_reorder_pics`.
    fn bump_for_reorder(&mut self) -> Vec<DecodedFrame> {
        let mut output = Vec::new();
        while self.num_needs_output() > self.max_num_reorder_pics {
            if let Some(f) = self.bump_one() {
                output.push(f);
            } else {
                break;
            }
        }
        output
    }

    /// Remove all entries that are neither needed for output nor
    /// used as a reference.
    fn evict_unused(&mut self) {
        self.entries.retain(|e| !e.is_evictable());
    }

    /// Forcibly evict the oldest already-output reference frame.
    ///
    /// This is a last resort when the DPB is full of reference frames
    /// that have already been output but are still marked as references.
    /// Returns `true` if a frame was evicted.
    fn force_evict_oldest_ref(&mut self) -> bool {
        // Prefer to evict already-output short-term references first,
        // then long-term, by decode order (oldest first).
        let idx = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.needed_for_output && e.is_short_term_ref())
            .min_by_key(|(_, e)| e.decode_order)
            .map(|(i, _)| i)
            .or_else(|| {
                self.entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| !e.needed_for_output && e.is_long_term_ref())
                    .min_by_key(|(_, e)| e.decode_order)
                    .map(|(i, _)| i)
            })
            .or_else(|| {
                // Absolute last resort: evict oldest reference even if
                // still needed for output.
                self.entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.is_reference())
                    .min_by_key(|(_, e)| e.decode_order)
                    .map(|(i, _)| i)
            });

        if let Some(i) = idx {
            log::debug!(
                "force-evicting DPB entry poc={} to make room",
                self.entries[i].pic_order_cnt
            );
            self.entries.remove(i);
            true
        } else {
            false
        }
    }

    /// Drain all pictures for output (in POC order), clearing output flags
    /// and removing non-reference pictures.
    fn drain_all_for_output(&mut self) -> Vec<DecodedFrame> {
        // Sort output-pending entries by POC.
        let mut output_indices: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.needed_for_output)
            .map(|(i, _)| i)
            .collect();
        output_indices.sort_by_key(|&i| self.entries[i].pic_order_cnt);

        let mut output = Vec::with_capacity(output_indices.len());
        for &idx in &output_indices {
            self.entries[idx].needed_for_output = false;
            output.push(self.entries[idx].frame.clone());
        }

        // Remove evictable entries.
        self.evict_unused();

        output
    }
}

impl Default for Dpb {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// DPB sizing helpers
// ---------------------------------------------------------------------------

/// Compute the maximum number of DPB frames for a given HEVC level and
/// picture size.
///
/// This follows Table A.8 from ITU-T H.265.  The `max_dpb_size` is
/// `min(MaxDpbSize, max_dec_pic_buffering + 1)` where `MaxDpbSize` is
/// derived from the level's `MaxLumaPs` and the picture's luma sample
/// count.
///
/// # Arguments
///
/// * `level_idc` – `general_level_idc` from the SPS profile-tier-level
///   (e.g. 120 for Level 4.0).
/// * `pic_size_in_samples` – total luma samples (`width * height`).
/// * `sps_max_dec_pic_buffering` – `sps_max_dec_pic_buffering_minus1[HighestTid] + 1`.
pub fn max_dpb_frames(
    level_idc: u8,
    pic_size_in_samples: u64,
    sps_max_dec_pic_buffering: u32,
) -> usize {
    let max_luma_ps: u64 = max_luma_ps_for_level(level_idc);

    let max_dpb_size = if pic_size_in_samples == 0 {
        ABSOLUTE_MAX_DPB_FRAMES
    } else if pic_size_in_samples <= (max_luma_ps >> 2) {
        // PicSizeInSamplesY <= MaxLumaPs / 4: MaxDpbSize = Min(4 * maxDpbReq, 16)
        std::cmp::min(4 * 6, ABSOLUTE_MAX_DPB_FRAMES) // 6 is a typical maxDpbReq
    } else if pic_size_in_samples <= (max_luma_ps >> 1) {
        std::cmp::min(2 * 6, ABSOLUTE_MAX_DPB_FRAMES)
    } else if pic_size_in_samples <= (3 * max_luma_ps / 4) {
        std::cmp::min((4 * 6) / 3, ABSOLUTE_MAX_DPB_FRAMES)
    } else {
        6 // Default maxDpbReq for single-layer
    };

    // The effective DPB size is the minimum of the level-derived max and
    // the SPS-signalled max.
    std::cmp::min(
        max_dpb_size,
        std::cmp::max(sps_max_dec_pic_buffering as usize, 1),
    )
    .min(ABSOLUTE_MAX_DPB_FRAMES)
}

/// Return `MaxLumaPs` for a given `general_level_idc` (Table A.8).
fn max_luma_ps_for_level(level_idc: u8) -> u64 {
    match level_idc {
        // Level 1
        30 => 36_864,
        // Level 2
        60 => 122_880,
        // Level 2.1
        63 => 245_760,
        // Level 3
        90 => 552_960,
        // Level 3.1
        93 => 983_040,
        // Level 4
        120 => 2_228_224,
        // Level 4.1
        123 => 2_228_224,
        // Level 5
        150 => 8_912_896,
        // Level 5.1
        153 => 8_912_896,
        // Level 5.2
        156 => 8_912_896,
        // Level 6
        180 => 35_651_584,
        // Level 6.1
        183 => 35_651_584,
        // Level 6.2
        186 => 35_651_584,
        // Unknown level – use a generous default.
        _ => 8_912_896,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::DecodedFrame;
    use crate::pixel::ColourMatrix;

    /// Create a simple test frame with given POC, reference flag, and IRAP flag.
    fn test_frame(poc: i32, is_ref: bool, is_irap: bool) -> DecodedFrame {
        let y = vec![128u8; 16 * 16];
        let u = vec![128u8; 8 * 8];
        let v = vec![128u8; 8 * 8];
        DecodedFrame::from_yuv420p(y, u, v, 16, 16)
            .with_frame_num(poc as u32)
            .with_pic_order_cnt(poc)
            .with_is_reference(is_ref)
            .with_is_irap(is_irap)
            .with_picture_type(if is_irap {
                PictureType::I
            } else {
                PictureType::P
            })
            .with_colour_matrix(ColourMatrix::Bt709)
    }

    #[test]
    fn test_new_dpb_is_empty() {
        let dpb = Dpb::new();
        assert!(dpb.is_empty());
        assert_eq!(dpb.len(), 0);
        assert!(!dpb.is_full());
        assert_eq!(dpb.max_size(), DEFAULT_DPB_SIZE);
    }

    #[test]
    fn test_insert_single_frame() {
        let mut dpb = Dpb::new();
        let frame = test_frame(0, true, true);
        let output = dpb.insert(frame).unwrap();
        // IDR flushes DPB first (empty, so no output from flush), then inserts.
        // No reorder bumping since max_num_reorder_pics defaults to 0.
        // But we have 1 output-pending > 0 max_reorder, so it gets bumped.
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].pic_order_cnt(), 0);
    }

    #[test]
    fn test_insert_multiple_no_reorder() {
        let mut dpb = Dpb::with_max_size(4);
        dpb.set_max_num_reorder_pics(0);

        let f0 = test_frame(0, true, true);
        let out0 = dpb.insert(f0).unwrap();
        assert_eq!(out0.len(), 1); // bumped immediately (reorder = 0)

        let f1 = test_frame(1, true, false);
        let out1 = dpb.insert(f1).unwrap();
        assert_eq!(out1.len(), 1);
        assert_eq!(out1[0].pic_order_cnt(), 1);
    }

    #[test]
    fn test_reorder_buffering() {
        let mut dpb = Dpb::with_max_size(6);
        dpb.set_max_num_reorder_pics(2);

        // Insert 3 non-IDR frames (they won't trigger IDR flush).
        let f0 = test_frame(0, true, false);
        let f1 = test_frame(2, true, false);
        let f2 = test_frame(1, true, false);

        let out0 = dpb.insert(f0).unwrap();
        assert!(out0.is_empty()); // reorder allows 2 pending

        let out1 = dpb.insert(f1).unwrap();
        assert!(out1.is_empty()); // 2 pending, still within limit

        let out2 = dpb.insert(f2).unwrap();
        // Now 3 pending > 2 max_reorder → bump lowest POC (0)
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0].pic_order_cnt(), 0);
    }

    #[test]
    fn test_idr_flushes_dpb() {
        let mut dpb = Dpb::with_max_size(6);
        dpb.set_max_num_reorder_pics(4);

        let f0 = test_frame(0, true, false);
        let f1 = test_frame(1, true, false);
        dpb.insert(f0).unwrap();
        dpb.insert(f1).unwrap();

        // Insert an IDR – should flush all pending.
        let idr = test_frame(2, true, true);
        let output = dpb.insert(idr).unwrap();
        // Should get f0 and f1 flushed (POC order), then the IDR itself bumped
        // because max_reorder_pics=4 but there might be bumping from flush + reorder.
        // Actually: IDR flush outputs f0, f1. Then IDR inserted. Then reorder
        // check: 1 pending <= 4, so IDR stays.
        assert!(output.len() >= 2);
        // First two should be in POC order.
        assert_eq!(output[0].pic_order_cnt(), 0);
        assert_eq!(output[1].pic_order_cnt(), 1);
    }

    #[test]
    fn test_dpb_overflow_bumps() {
        let mut dpb = Dpb::with_max_size(2);
        dpb.set_max_num_reorder_pics(2);

        let f0 = test_frame(0, true, false);
        let f1 = test_frame(1, true, false);
        let f2 = test_frame(2, true, false);

        dpb.insert(f0).unwrap();
        dpb.insert(f1).unwrap();

        // DPB is full (2/2). Inserting f2 should bump f0.
        let output = dpb.insert(f2).unwrap();
        assert!(!output.is_empty());
        assert_eq!(output[0].pic_order_cnt(), 0);
    }

    #[test]
    fn test_flush_returns_poc_order() {
        let mut dpb = Dpb::with_max_size(6);
        dpb.set_max_num_reorder_pics(6);

        // Insert in decode order: POC 4, 2, 0, 3, 1
        for &poc in &[4i32, 2, 0, 3, 1] {
            let f = test_frame(poc, true, false);
            dpb.insert(f).unwrap();
        }

        let output = dpb.flush();
        let pocs: Vec<i32> = output.iter().map(|f| f.pic_order_cnt()).collect();
        assert_eq!(pocs, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn test_mark_unused_by_poc() {
        let mut dpb = Dpb::with_max_size(6);
        dpb.set_max_num_reorder_pics(6);

        let f = test_frame(5, true, false);
        dpb.insert(f).unwrap();

        assert_eq!(dpb.num_short_term_refs(), 1);
        dpb.mark_unused_by_poc(5);
        assert_eq!(dpb.num_short_term_refs(), 0);
        assert_eq!(dpb.num_refs(), 0);
    }

    #[test]
    fn test_mark_long_term_ref() {
        let mut dpb = Dpb::with_max_size(6);
        dpb.set_max_num_reorder_pics(6);

        let f = test_frame(3, true, false);
        dpb.insert(f).unwrap();

        assert_eq!(dpb.num_short_term_refs(), 1);
        assert_eq!(dpb.num_long_term_refs(), 0);

        dpb.mark_long_term_ref_by_poc(3);
        assert_eq!(dpb.num_short_term_refs(), 0);
        assert_eq!(dpb.num_long_term_refs(), 1);
    }

    #[test]
    fn test_mark_all_unused() {
        let mut dpb = Dpb::with_max_size(6);
        dpb.set_max_num_reorder_pics(6);

        for poc in 0..3 {
            let f = test_frame(poc, true, false);
            dpb.insert(f).unwrap();
        }

        assert_eq!(dpb.num_refs(), 3);
        dpb.mark_all_unused();
        assert_eq!(dpb.num_refs(), 0);
    }

    #[test]
    fn test_find_short_term_ref() {
        let mut dpb = Dpb::with_max_size(6);
        dpb.set_max_num_reorder_pics(6);

        let f = test_frame(7, true, false);
        dpb.insert(f).unwrap();

        assert!(dpb.find_short_term_ref(7).is_some());
        assert!(dpb.find_short_term_ref(8).is_none());

        // After marking unused, it should no longer be found.
        dpb.mark_unused_by_poc(7);
        assert!(dpb.find_short_term_ref(7).is_none());
    }

    #[test]
    fn test_find_ref_by_poc() {
        let mut dpb = Dpb::with_max_size(6);
        dpb.set_max_num_reorder_pics(6);

        let f = test_frame(5, true, false);
        dpb.insert(f).unwrap();

        assert!(dpb.find_ref_by_poc(5).is_some());
        assert_eq!(dpb.find_ref_by_poc(5).unwrap().pic_order_cnt(), 5);
    }

    #[test]
    fn test_build_ref_list_0() {
        let mut dpb = Dpb::with_max_size(8);
        dpb.set_max_num_reorder_pics(8);

        for &poc in &[0, 2, 4, 6] {
            let f = test_frame(poc, true, false);
            dpb.insert(f).unwrap();
        }

        let list = dpb.build_ref_list_0(4);
        // Before current (4): POC 2, 0 (descending)
        // After current (4): POC 6 (ascending)
        let pocs: Vec<i32> = list.iter().map(|f| f.pic_order_cnt()).collect();
        assert_eq!(pocs, vec![2, 0, 6]);
    }

    #[test]
    fn test_build_ref_list_1() {
        let mut dpb = Dpb::with_max_size(8);
        dpb.set_max_num_reorder_pics(8);

        for &poc in &[0, 2, 4, 6] {
            let f = test_frame(poc, true, false);
            dpb.insert(f).unwrap();
        }

        let list = dpb.build_ref_list_1(4);
        // After current (4): POC 6 (ascending)
        // Before current (4): POC 2, 0 (descending)
        let pocs: Vec<i32> = list.iter().map(|f| f.pic_order_cnt()).collect();
        assert_eq!(pocs, vec![6, 2, 0]);
    }

    #[test]
    fn test_set_max_size_bumps_excess() {
        let mut dpb = Dpb::with_max_size(6);
        dpb.set_max_num_reorder_pics(6);

        for poc in 0..4 {
            let f = test_frame(poc, true, false);
            dpb.insert(f).unwrap();
        }

        assert_eq!(dpb.len(), 4);

        // Shrink to 2 – should bump 2 frames.
        let bumped = dpb.set_max_size(2);
        assert!(bumped.len() >= 2);
        assert!(dpb.len() <= 2);
    }

    #[test]
    fn test_clear_empties_dpb() {
        let mut dpb = Dpb::with_max_size(6);
        dpb.set_max_num_reorder_pics(6);

        let f = test_frame(0, true, false);
        dpb.insert(f).unwrap();
        assert!(!dpb.is_empty());

        dpb.clear();
        assert!(dpb.is_empty());
        assert_eq!(dpb.len(), 0);
    }

    #[test]
    fn test_max_dpb_frames() {
        // Level 4.0 (120), 1920x1080 = 2,073,600 samples, sps_max = 5
        let size = max_dpb_frames(120, 1920 * 1080, 5);
        assert!(size <= ABSOLUTE_MAX_DPB_FRAMES);
        assert!(size >= 1);
        assert!(size <= 5);

        // Level 3.1, 1280x720, sps_max = 4
        let size = max_dpb_frames(93, 1280 * 720, 4);
        assert!(size >= 1);
        assert!(size <= 4);
    }

    #[test]
    fn test_max_dpb_frames_zero_pic_size() {
        let size = max_dpb_frames(120, 0, 6);
        assert!(size <= ABSOLUTE_MAX_DPB_FRAMES);
    }

    #[test]
    fn test_max_luma_ps_for_level() {
        assert_eq!(max_luma_ps_for_level(120), 2_228_224); // Level 4.0
        assert_eq!(max_luma_ps_for_level(150), 8_912_896); // Level 5.0
        assert_eq!(max_luma_ps_for_level(180), 35_651_584); // Level 6.0
    }

    #[test]
    fn test_evict_unused_after_output_and_unref() {
        let mut dpb = Dpb::with_max_size(6);
        dpb.set_max_num_reorder_pics(0); // immediate output

        // Insert a non-reference frame.
        let f = test_frame(0, false, false);
        let output = dpb.insert(f).unwrap();
        // Should be bumped immediately (reorder=0) and since it's not a
        // reference, it should be evicted.
        assert_eq!(output.len(), 1);
        assert_eq!(dpb.len(), 0);
    }
}

//! Decoded Picture Buffer (DPB) for H.264 reference frame management.
//!
//! The DPB stores decoded pictures that may be needed as references for
//! inter-prediction of future frames (P and B slices), and also buffers
//! frames for reordering from decode order into display (picture order
//! count) order.
//!
//! The maximum DPB size is derived from the H.264 level limits specified
//! in the Sequence Parameter Set (SPS).  When the buffer is full, frames
//! are bumped (output) according to the sliding-window or MMCO marking
//! process defined in the H.264 specification (sub-clauses 8.2.5.3 and
//! 8.2.5.4).

use crate::error::{DecodeError, DecodeResult};
use crate::frame::DecodedFrame;

/// Maximum DPB size in frames allowed by any H.264 level (Level 6.2).
const ABSOLUTE_MAX_DPB_FRAMES: usize = 16;

/// Default DPB size when no SPS-derived value is available.
const DEFAULT_DPB_SIZE: usize = 16;

/// State of a picture stored in the DPB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReferenceStatus {
    /// The picture is not used for reference (only kept for display
    /// reordering purposes).
    Unused,
    /// The picture is used for short-term reference prediction.
    ShortTerm,
    /// The picture is used for long-term reference prediction.
    LongTerm,
}

/// A single entry in the DPB, pairing a decoded frame with its reference
/// marking state and output status.
#[derive(Debug, Clone)]
struct DpbEntry {
    /// The decoded picture data.
    frame: DecodedFrame,
    /// Current reference marking.
    reference: ReferenceStatus,
    /// Whether this picture has already been output (bumped) for display.
    output: bool,
    /// Whether this picture is still needed for output (i.e. it has not
    /// been bumped yet and is waiting for its display-order turn).
    needed_for_output: bool,
    /// Frame number from the slice header (`frame_num`).
    frame_num: u32,
    /// Picture order count used for display ordering.
    pic_order_cnt: i32,
    /// Long-term frame index (only meaningful when `reference == LongTerm`).
    long_term_frame_idx: Option<u32>,
}

impl DpbEntry {
    fn new(frame: DecodedFrame) -> Self {
        let frame_num = frame.frame_num();
        let pic_order_cnt = frame.pic_order_cnt();
        let reference = if frame.is_reference() {
            ReferenceStatus::ShortTerm
        } else {
            ReferenceStatus::Unused
        };

        Self {
            frame,
            reference,
            output: false,
            needed_for_output: true,
            frame_num,
            pic_order_cnt,
            long_term_frame_idx: None,
        }
    }

    /// Returns `true` if this entry is being used for reference (either
    /// short-term or long-term).
    fn is_reference(&self) -> bool {
        self.reference != ReferenceStatus::Unused
    }

    /// Returns `true` if this entry can be evicted from the DPB (it has
    /// been output and is not used for reference).
    fn is_evictable(&self) -> bool {
        !self.is_reference() && !self.needed_for_output
    }
}

/// The Decoded Picture Buffer.
///
/// Manages reference frames for inter-prediction and reorders frames from
/// decode order into display order based on picture order count (POC).
///
/// # Usage
///
/// ```ignore
/// let mut dpb = Dpb::new();
/// dpb.set_max_size(4);
///
/// // After decoding each picture:
/// let output = dpb.insert(decoded_frame)?;
/// for frame in output {
///     // send to display / downstream encoder
/// }
///
/// // At end of stream:
/// let remaining = dpb.flush();
/// ```
#[derive(Debug)]
pub struct Dpb {
    /// The stored decoded pictures.
    entries: Vec<DpbEntry>,
    /// Maximum number of frames the DPB can hold, derived from SPS level.
    max_size: usize,
    /// Maximum number of frames that may be reordered before output.
    /// Derived from `max_num_reorder_frames` in the SPS VUI, or defaults
    /// to `max_size - 1` when not signalled.
    max_num_reorder_frames: usize,
}

impl Dpb {
    /// Create a new, empty DPB with the default maximum size.
    pub fn new() -> Self {
        Self {
            entries: Vec::with_capacity(DEFAULT_DPB_SIZE),
            max_size: DEFAULT_DPB_SIZE,
            max_num_reorder_frames: DEFAULT_DPB_SIZE.saturating_sub(1),
        }
    }

    /// Create a DPB with a specific maximum frame capacity.
    pub fn with_max_size(max_size: usize) -> Self {
        let max_size = max_size.clamp(1, ABSOLUTE_MAX_DPB_FRAMES);
        Self {
            entries: Vec::with_capacity(max_size),
            max_size,
            max_num_reorder_frames: max_size.saturating_sub(1),
        }
    }

    /// Set the maximum DPB size (in frames).
    ///
    /// This is typically called when a new SPS is activated so that the DPB
    /// capacity matches the level constraints.  If the new size is smaller
    /// than the current number of stored frames, excess frames are bumped.
    pub fn set_max_size(&mut self, max_size: usize) -> Vec<DecodedFrame> {
        self.max_size = max_size.clamp(1, ABSOLUTE_MAX_DPB_FRAMES);
        if self.max_num_reorder_frames >= self.max_size {
            self.max_num_reorder_frames = self.max_size.saturating_sub(1);
        }
        self.bump_until_within_capacity()
    }

    /// Set the maximum reorder depth.
    ///
    /// Derived from `max_num_reorder_frames` in the SPS VUI parameters.
    /// A value of 0 means no reordering (frames are output immediately in
    /// decode order).
    pub fn set_max_num_reorder_frames(&mut self, n: usize) {
        self.max_num_reorder_frames = n.min(self.max_size);
    }

    /// Current maximum DPB capacity in frames.
    #[inline]
    pub fn max_size(&self) -> usize {
        self.max_size
    }

    /// Number of frames currently stored in the DPB.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if the DPB contains no frames.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns `true` if the DPB is at maximum capacity.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.entries.len() >= self.max_size
    }

    /// Number of short-term reference frames currently in the DPB.
    pub fn num_short_term_refs(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.reference == ReferenceStatus::ShortTerm)
            .count()
    }

    /// Number of long-term reference frames currently in the DPB.
    pub fn num_long_term_refs(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.reference == ReferenceStatus::LongTerm)
            .count()
    }

    /// Total number of reference frames (short-term + long-term).
    pub fn num_refs(&self) -> usize {
        self.entries.iter().filter(|e| e.is_reference()).count()
    }

    /// Insert a newly decoded frame into the DPB.
    ///
    /// If the DPB is full, frames are bumped (output in display order) to
    /// make room.  Returns a list of frames that should be output for
    /// display, sorted by picture order count.
    ///
    /// If the frame is an IDR, the DPB is cleared first and all previously
    /// stored frames are flushed to output.
    pub fn insert(&mut self, frame: DecodedFrame) -> DecodeResult<Vec<DecodedFrame>> {
        let mut output = Vec::new();

        // IDR pictures cause an instantaneous decoder refresh: all
        // previously-decoded reference pictures are marked as "unused for
        // reference" (H.264 spec sub-clause 8.2.5.2), then all frames
        // are flushed for output and the DPB is cleared.
        if frame.is_idr() {
            for entry in &mut self.entries {
                entry.reference = ReferenceStatus::Unused;
            }
            output.extend(self.drain_all_for_output());
        }

        // Ensure there is room in the DPB.
        if self.entries.len() >= self.max_size {
            let bumped = self.bump_one();
            if let Some(f) = bumped {
                output.push(f);
            }

            // If still full after bumping, try evicting an unreferenced,
            // already-output frame.
            if self.entries.len() >= self.max_size {
                self.evict_unused();

                if self.entries.len() >= self.max_size {
                    return Err(DecodeError::DpbOverflow {
                        capacity: self.max_size,
                        frame_num: frame.frame_num(),
                    });
                }
            }
        }

        let entry = DpbEntry::new(frame);
        self.entries.push(entry);

        // Check if we should bump additional frames based on the reorder
        // constraint: if the number of frames needing output exceeds
        // max_num_reorder_frames + 1, bump the lowest-POC frame.
        output.extend(self.bump_for_reorder());

        Ok(output)
    }

    /// Flush all remaining frames from the DPB in display order.
    ///
    /// Call this at end-of-stream to retrieve any buffered frames that have
    /// not yet been output.
    pub fn flush(&mut self) -> Vec<DecodedFrame> {
        self.drain_all_for_output()
    }

    /// Clear the DPB entirely, discarding all frames without outputting
    /// them.  This is a hard reset.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Find a short-term reference frame by `frame_num`.
    ///
    /// Returns `None` if no matching short-term reference is present.
    pub fn find_short_term_ref(&self, frame_num: u32) -> Option<&DecodedFrame> {
        self.entries
            .iter()
            .find(|e| e.reference == ReferenceStatus::ShortTerm && e.frame_num == frame_num)
            .map(|e| &e.frame)
    }

    /// Find a long-term reference frame by long-term frame index.
    pub fn find_long_term_ref(&self, long_term_frame_idx: u32) -> Option<&DecodedFrame> {
        self.entries
            .iter()
            .find(|e| {
                e.reference == ReferenceStatus::LongTerm
                    && e.long_term_frame_idx == Some(long_term_frame_idx)
            })
            .map(|e| &e.frame)
    }

    /// Find a reference frame (short-term or long-term) by picture order count.
    pub fn find_ref_by_poc(&self, poc: i32) -> Option<&DecodedFrame> {
        self.entries
            .iter()
            .find(|e| e.is_reference() && e.pic_order_cnt == poc)
            .map(|e| &e.frame)
    }

    /// Build the reference picture list 0 (for P and B slices).
    ///
    /// For P slices, list 0 contains short-term references sorted by
    /// descending `frame_num` followed by long-term references sorted by
    /// ascending long-term frame index.
    pub fn build_ref_list_0(&self, current_frame_num: u32) -> Vec<&DecodedFrame> {
        let mut short_term: Vec<&DpbEntry> = self
            .entries
            .iter()
            .filter(|e| e.reference == ReferenceStatus::ShortTerm)
            .collect();

        // Sort short-term refs by descending PicNum (approximated by
        // frame_num distance from current).  Frames with smaller distance
        // come first.
        short_term.sort_by(|a, b| {
            let dist_a = current_frame_num.wrapping_sub(a.frame_num);
            let dist_b = current_frame_num.wrapping_sub(b.frame_num);
            dist_a.cmp(&dist_b)
        });

        let mut long_term: Vec<&DpbEntry> = self
            .entries
            .iter()
            .filter(|e| e.reference == ReferenceStatus::LongTerm)
            .collect();

        long_term.sort_by_key(|e| e.long_term_frame_idx.unwrap_or(u32::MAX));

        let mut list: Vec<&DecodedFrame> = Vec::with_capacity(short_term.len() + long_term.len());
        for e in &short_term {
            list.push(&e.frame);
        }
        for e in &long_term {
            list.push(&e.frame);
        }

        list
    }

    /// Build the reference picture list 1 (for B slices).
    ///
    /// List 1 contains short-term references sorted by ascending POC
    /// (for pictures after the current in display order) then descending
    /// POC (for pictures before), followed by long-term references.
    pub fn build_ref_list_1(&self, current_poc: i32) -> Vec<&DecodedFrame> {
        let mut after: Vec<&DpbEntry> = self
            .entries
            .iter()
            .filter(|e| e.reference == ReferenceStatus::ShortTerm && e.pic_order_cnt > current_poc)
            .collect();
        after.sort_by_key(|e| e.pic_order_cnt);

        let mut before: Vec<&DpbEntry> = self
            .entries
            .iter()
            .filter(|e| e.reference == ReferenceStatus::ShortTerm && e.pic_order_cnt <= current_poc)
            .collect();
        before.sort_by(|a, b| b.pic_order_cnt.cmp(&a.pic_order_cnt));

        let mut long_term: Vec<&DpbEntry> = self
            .entries
            .iter()
            .filter(|e| e.reference == ReferenceStatus::LongTerm)
            .collect();
        long_term.sort_by_key(|e| e.long_term_frame_idx.unwrap_or(u32::MAX));

        let mut list: Vec<&DecodedFrame> =
            Vec::with_capacity(after.len() + before.len() + long_term.len());
        for e in &after {
            list.push(&e.frame);
        }
        for e in &before {
            list.push(&e.frame);
        }
        for e in &long_term {
            list.push(&e.frame);
        }

        list
    }

    // ------------------------------------------------------------------
    // Sliding window reference marking (sub-clause 8.2.5.3)
    // ------------------------------------------------------------------

    /// Apply the sliding-window marking process.
    ///
    /// When the number of short-term reference frames exceeds the maximum
    /// allowed by the SPS (`max_num_ref_frames`), the oldest short-term
    /// reference is marked as "unused for reference".
    pub fn sliding_window_mark(&mut self, max_num_ref_frames: usize) {
        while self.num_short_term_refs() + self.num_long_term_refs() > max_num_ref_frames
            && self.num_short_term_refs() > 0
        {
            // Find the short-term ref with the smallest frame_num (oldest).
            if let Some(idx) = self.oldest_short_term_ref_index() {
                self.entries[idx].reference = ReferenceStatus::Unused;
            } else {
                break;
            }
        }
    }

    // ------------------------------------------------------------------
    // MMCO (Memory Management Control Operations) — sub-clause 8.2.5.4
    // ------------------------------------------------------------------

    /// Mark a specific short-term reference as "unused for reference" by
    /// `frame_num`.
    pub fn mmco_mark_short_term_unused(&mut self, frame_num: u32) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.reference == ReferenceStatus::ShortTerm && e.frame_num == frame_num)
        {
            entry.reference = ReferenceStatus::Unused;
        }
    }

    /// Mark a specific long-term reference as "unused for reference" by
    /// long-term frame index.
    pub fn mmco_mark_long_term_unused(&mut self, long_term_frame_idx: u32) {
        if let Some(entry) = self.entries.iter_mut().find(|e| {
            e.reference == ReferenceStatus::LongTerm
                && e.long_term_frame_idx == Some(long_term_frame_idx)
        }) {
            entry.reference = ReferenceStatus::Unused;
            entry.long_term_frame_idx = None;
        }
    }

    /// Promote a short-term reference to long-term with the given index.
    pub fn mmco_assign_long_term(
        &mut self,
        frame_num: u32,
        long_term_frame_idx: u32,
    ) -> DecodeResult<()> {
        // First, unmark any existing long-term ref with the same index.
        self.mmco_mark_long_term_unused(long_term_frame_idx);

        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.reference == ReferenceStatus::ShortTerm && e.frame_num == frame_num)
        {
            entry.reference = ReferenceStatus::LongTerm;
            entry.long_term_frame_idx = Some(long_term_frame_idx);
            Ok(())
        } else {
            Err(DecodeError::MissingReference(format!(
                "short-term ref with frame_num={frame_num} not found for MMCO long-term assignment"
            )))
        }
    }

    /// Mark all reference pictures as "unused for reference" (MMCO 5).
    ///
    /// Returns frames that were waiting for output.
    pub fn mmco_clear_all_refs(&mut self) -> Vec<DecodedFrame> {
        for entry in &mut self.entries {
            entry.reference = ReferenceStatus::Unused;
        }
        // Bump all frames that are needed for output since there are no
        // more references.
        self.drain_all_for_output()
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Find the index of the oldest (smallest `frame_num`) short-term ref.
    fn oldest_short_term_ref_index(&self) -> Option<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.reference == ReferenceStatus::ShortTerm)
            .min_by_key(|(_, e)| e.frame_num)
            .map(|(i, _)| i)
    }

    /// Bump (output) the frame with the smallest POC that is still needed
    /// for output.  Returns `None` if no frame is waiting for output.
    fn bump_one(&mut self) -> Option<DecodedFrame> {
        let idx = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.needed_for_output)
            .min_by_key(|(_, e)| e.pic_order_cnt)
            .map(|(i, _)| i);

        if let Some(i) = idx {
            self.entries[i].needed_for_output = false;
            self.entries[i].output = true;

            let frame = self.entries[i].frame.clone();

            // If the entry is also not a reference, remove it entirely.
            if !self.entries[i].is_reference() {
                self.entries.remove(i);
            }

            Some(frame)
        } else {
            None
        }
    }

    /// Bump frames until the DPB is within capacity.
    fn bump_until_within_capacity(&mut self) -> Vec<DecodedFrame> {
        let mut output = Vec::new();
        while self.entries.len() > self.max_size {
            if let Some(frame) = self.bump_one() {
                output.push(frame);
            } else {
                // If we can't bump any more, force-evict unused entries.
                self.evict_unused();
                break;
            }
        }
        output
    }

    /// Bump frames that exceed the reorder window.
    fn bump_for_reorder(&mut self) -> Vec<DecodedFrame> {
        let mut output = Vec::new();
        loop {
            let num_needing_output = self.entries.iter().filter(|e| e.needed_for_output).count();

            if num_needing_output > self.max_num_reorder_frames + 1 {
                if let Some(frame) = self.bump_one() {
                    output.push(frame);
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        output
    }

    /// Remove entries that are neither references nor needed for output.
    fn evict_unused(&mut self) {
        self.entries.retain(|e| !e.is_evictable());
    }

    /// Drain all frames that are needed for output, sorted by POC.
    fn drain_all_for_output(&mut self) -> Vec<DecodedFrame> {
        // Collect frames needing output, sorted by POC.
        let mut pending: Vec<(usize, i32)> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.needed_for_output)
            .map(|(i, e)| (i, e.pic_order_cnt))
            .collect();
        pending.sort_by_key(|(_, poc)| *poc);

        let mut output = Vec::with_capacity(pending.len());
        for &(i, _) in &pending {
            self.entries[i].needed_for_output = false;
            self.entries[i].output = true;
            output.push(self.entries[i].frame.clone());
        }

        // Remove all entries that are now fully evictable.
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
// Compute max DPB size from H.264 level
// ---------------------------------------------------------------------------

/// Compute the maximum DPB size in frames for a given H.264 level and
/// picture dimensions.
///
/// The DPB size in bytes is level-dependent (Table A-1 in the H.264 spec).
/// Dividing by the frame size gives the number of frames.  The result is
/// clamped to [`ABSOLUTE_MAX_DPB_FRAMES`] (16).
pub fn max_dpb_frames(level_idc: u8, pic_width_mbs: u32, pic_height_mbs: u32) -> usize {
    let max_dpb_mbs = match level_idc {
        10 => 396,
        11 => 900,
        12 => 2376,
        13 => 2376,
        20 => 2376,
        21 => 4752,
        22 => 8100,
        30 => 8100,
        31 => 18000,
        32 => 20480,
        40 | 41 => 32768,
        42 => 34816,
        50 => 110400,
        51 => 184320,
        52 => 184320,
        60..=62 => 696320,
        // For unknown levels, use a conservative default.
        _ => 32768,
    };

    let frame_mbs = pic_width_mbs as usize * pic_height_mbs as usize;
    if frame_mbs == 0 {
        return DEFAULT_DPB_SIZE.min(ABSOLUTE_MAX_DPB_FRAMES);
    }

    let frames = max_dpb_mbs / frame_mbs;
    frames.clamp(1, ABSOLUTE_MAX_DPB_FRAMES)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{DecodedFrame, PictureType};

    /// Helper to create a minimal test frame.
    fn test_frame(frame_num: u32, poc: i32, is_ref: bool, is_idr: bool) -> DecodedFrame {
        let w = 4u32;
        let h = 4u32;
        let y = vec![128u8; (w * h) as usize];
        let u = vec![128u8; ((w / 2) * (h / 2)) as usize];
        let v = vec![128u8; ((w / 2) * (h / 2)) as usize];

        DecodedFrame::from_yuv420p(y, u, v, w, h)
            .with_frame_num(frame_num)
            .with_pic_order_cnt(poc)
            .with_is_reference(is_ref)
            .with_is_idr(is_idr)
            .with_picture_type(if is_idr {
                PictureType::I
            } else {
                PictureType::P
            })
    }

    #[test]
    fn test_new_dpb_is_empty() {
        let dpb = Dpb::new();
        assert!(dpb.is_empty());
        assert_eq!(dpb.len(), 0);
        assert!(!dpb.is_full());
    }

    #[test]
    fn test_insert_single_frame() {
        let mut dpb = Dpb::with_max_size(4);
        let frame = test_frame(0, 0, true, true);
        let output = dpb.insert(frame).unwrap();

        // IDR flushes first (DPB was empty, so nothing to flush), then
        // the frame is stored.
        assert_eq!(dpb.len(), 1);
        assert!(output.is_empty()); // no frames bumped
    }

    #[test]
    fn test_idr_flushes_dpb() {
        let mut dpb = Dpb::with_max_size(4);

        // Insert some reference frames.
        dpb.insert(test_frame(0, 0, true, true)).unwrap();
        dpb.insert(test_frame(1, 2, true, false)).unwrap();
        dpb.insert(test_frame(2, 4, true, false)).unwrap();
        assert_eq!(dpb.len(), 3);

        // Insert an IDR – should flush all previous frames.
        let output = dpb.insert(test_frame(0, 6, true, true)).unwrap();
        assert!(!output.is_empty());

        // After IDR, DPB should only contain the new IDR frame.
        assert_eq!(dpb.len(), 1);
    }

    #[test]
    fn test_dpb_overflow_bumps_frames() {
        let mut dpb = Dpb::with_max_size(2);
        dpb.set_max_num_reorder_frames(2);

        dpb.insert(test_frame(0, 0, false, true)).unwrap();
        dpb.insert(test_frame(1, 2, false, false)).unwrap();

        // DPB is full (size=2). Inserting another should bump the
        // lowest-POC frame.
        let output = dpb.insert(test_frame(2, 4, false, false)).unwrap();
        assert!(!output.is_empty());
        assert_eq!(output[0].pic_order_cnt(), 0);
    }

    #[test]
    fn test_flush_returns_display_order() {
        let mut dpb = Dpb::with_max_size(8);
        dpb.set_max_num_reorder_frames(8);

        // Insert frames out of display order.
        dpb.insert(test_frame(0, 4, true, true)).unwrap();
        dpb.insert(test_frame(1, 0, true, false)).unwrap();
        dpb.insert(test_frame(2, 2, true, false)).unwrap();

        let output = dpb.flush();
        // Should come back sorted by POC: 0, 2, 4
        assert_eq!(output.len(), 3);
        assert_eq!(output[0].pic_order_cnt(), 0);
        assert_eq!(output[1].pic_order_cnt(), 2);
        assert_eq!(output[2].pic_order_cnt(), 4);
    }

    #[test]
    fn test_sliding_window_mark() {
        let mut dpb = Dpb::with_max_size(8);
        dpb.set_max_num_reorder_frames(8);

        dpb.insert(test_frame(0, 0, true, true)).unwrap();
        dpb.insert(test_frame(1, 2, true, false)).unwrap();
        dpb.insert(test_frame(2, 4, true, false)).unwrap();

        assert_eq!(dpb.num_short_term_refs(), 3);

        // Sliding window with max_num_ref_frames=2 should unmark the oldest.
        dpb.sliding_window_mark(2);
        assert_eq!(dpb.num_short_term_refs(), 2);
    }

    #[test]
    fn test_find_short_term_ref() {
        let mut dpb = Dpb::with_max_size(4);
        dpb.set_max_num_reorder_frames(4);

        dpb.insert(test_frame(0, 0, true, true)).unwrap();
        dpb.insert(test_frame(5, 2, true, false)).unwrap();

        assert!(dpb.find_short_term_ref(5).is_some());
        assert_eq!(dpb.find_short_term_ref(5).unwrap().frame_num(), 5);
        assert!(dpb.find_short_term_ref(99).is_none());
    }

    #[test]
    fn test_mmco_short_term_unused() {
        let mut dpb = Dpb::with_max_size(4);
        dpb.set_max_num_reorder_frames(4);

        dpb.insert(test_frame(0, 0, true, true)).unwrap();
        dpb.insert(test_frame(1, 2, true, false)).unwrap();

        assert_eq!(dpb.num_short_term_refs(), 2);
        dpb.mmco_mark_short_term_unused(0);
        assert_eq!(dpb.num_short_term_refs(), 1);
    }

    #[test]
    fn test_mmco_assign_long_term() {
        let mut dpb = Dpb::with_max_size(4);
        dpb.set_max_num_reorder_frames(4);

        dpb.insert(test_frame(0, 0, true, true)).unwrap();
        dpb.insert(test_frame(1, 2, true, false)).unwrap();

        assert_eq!(dpb.num_long_term_refs(), 0);
        dpb.mmco_assign_long_term(1, 0).unwrap();
        assert_eq!(dpb.num_long_term_refs(), 1);
        assert_eq!(dpb.num_short_term_refs(), 1);

        assert!(dpb.find_long_term_ref(0).is_some());
        assert_eq!(dpb.find_long_term_ref(0).unwrap().frame_num(), 1);
    }

    #[test]
    fn test_mmco_clear_all_refs() {
        let mut dpb = Dpb::with_max_size(4);
        dpb.set_max_num_reorder_frames(4);

        dpb.insert(test_frame(0, 0, true, true)).unwrap();
        dpb.insert(test_frame(1, 2, true, false)).unwrap();

        let output = dpb.mmco_clear_all_refs();
        assert_eq!(output.len(), 2);
        assert_eq!(dpb.num_refs(), 0);
        assert!(dpb.is_empty());
    }

    #[test]
    fn test_build_ref_list_0() {
        let mut dpb = Dpb::with_max_size(8);
        dpb.set_max_num_reorder_frames(8);

        dpb.insert(test_frame(0, 0, true, true)).unwrap();
        dpb.insert(test_frame(1, 2, true, false)).unwrap();
        dpb.insert(test_frame(2, 4, true, false)).unwrap();

        let list = dpb.build_ref_list_0(3);
        // Should be sorted by ascending distance from current frame_num=3:
        // frame_num=2 (dist=1), frame_num=1 (dist=2), frame_num=0 (dist=3)
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].frame_num(), 2);
        assert_eq!(list[1].frame_num(), 1);
        assert_eq!(list[2].frame_num(), 0);
    }

    #[test]
    fn test_build_ref_list_1() {
        let mut dpb = Dpb::with_max_size(8);
        dpb.set_max_num_reorder_frames(8);

        dpb.insert(test_frame(0, 0, true, true)).unwrap();
        dpb.insert(test_frame(1, 4, true, false)).unwrap();
        dpb.insert(test_frame(2, 8, true, false)).unwrap();

        // Current POC = 2. List 1: after (poc>2) ascending, then before (poc<=2) descending.
        let list = dpb.build_ref_list_1(2);
        assert_eq!(list.len(), 3);
        // After: POC 4, POC 8
        assert_eq!(list[0].pic_order_cnt(), 4);
        assert_eq!(list[1].pic_order_cnt(), 8);
        // Before: POC 0
        assert_eq!(list[2].pic_order_cnt(), 0);
    }

    #[test]
    fn test_max_dpb_frames_1080p_level31() {
        // 1920x1080 = 120x68 macroblocks = 8160 MBs
        // Level 3.1: MaxDpbMbs = 18000 => 18000/8160 = 2 frames
        let frames = max_dpb_frames(31, 120, 68);
        assert_eq!(frames, 2);
    }

    #[test]
    fn test_max_dpb_frames_720p_level31() {
        // 1280x720 = 80x45 macroblocks = 3600 MBs
        // Level 3.1: MaxDpbMbs = 18000 => 18000/3600 = 5 frames
        let frames = max_dpb_frames(31, 80, 45);
        assert_eq!(frames, 5);
    }

    #[test]
    fn test_max_dpb_frames_clamped_to_16() {
        // Very small frame at a high level could exceed 16.
        // 16x16 = 1x1 macroblock
        // Level 5.1: MaxDpbMbs = 184320 => 184320/1 >> 16 → clamped
        let frames = max_dpb_frames(51, 1, 1);
        assert_eq!(frames, ABSOLUTE_MAX_DPB_FRAMES);
    }

    #[test]
    fn test_max_dpb_frames_zero_dimensions() {
        let frames = max_dpb_frames(31, 0, 0);
        assert!(frames >= 1);
    }

    #[test]
    fn test_set_max_size_bumps_excess() {
        let mut dpb = Dpb::with_max_size(4);
        dpb.set_max_num_reorder_frames(4);

        dpb.insert(test_frame(0, 0, false, true)).unwrap();
        dpb.insert(test_frame(1, 2, false, false)).unwrap();
        dpb.insert(test_frame(2, 4, false, false)).unwrap();
        dpb.insert(test_frame(3, 6, false, false)).unwrap();
        assert_eq!(dpb.len(), 4);

        let bumped = dpb.set_max_size(2);
        // At least 2 frames should have been bumped.
        assert!(!bumped.is_empty());
        assert!(dpb.len() <= 2);
    }

    #[test]
    fn test_reorder_bump() {
        let mut dpb = Dpb::with_max_size(8);
        dpb.set_max_num_reorder_frames(1);

        // With max_num_reorder_frames=1, after inserting more than 2
        // frames needing output, the lowest-POC frame should be bumped.
        let out1 = dpb.insert(test_frame(0, 4, true, true)).unwrap();
        assert!(out1.is_empty());

        let out2 = dpb.insert(test_frame(1, 0, true, false)).unwrap();
        assert!(out2.is_empty());

        // Third insert: 3 needing output > max_reorder(1) + 1 = 2,
        // so the POC=0 frame should be bumped.
        let out3 = dpb.insert(test_frame(2, 2, true, false)).unwrap();
        assert!(!out3.is_empty());
        assert_eq!(out3[0].pic_order_cnt(), 0);
    }

    #[test]
    fn test_find_ref_by_poc() {
        let mut dpb = Dpb::with_max_size(4);
        dpb.set_max_num_reorder_frames(4);

        dpb.insert(test_frame(0, 0, true, true)).unwrap();
        dpb.insert(test_frame(1, 42, true, false)).unwrap();

        let found = dpb.find_ref_by_poc(42);
        assert!(found.is_some());
        assert_eq!(found.unwrap().frame_num(), 1);

        assert!(dpb.find_ref_by_poc(999).is_none());
    }
}

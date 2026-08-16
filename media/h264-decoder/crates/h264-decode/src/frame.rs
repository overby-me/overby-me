//! Decoded frame representation.
//!
//! A [`DecodedFrame`] holds the raw pixel data produced by the decoder for a
//! single picture, along with metadata such as dimensions, pixel format,
//! presentation timestamp, and per-plane accessors.

use crate::pixel::{
    ColourMatrix, PixelFormat, yuv420p_to_nv12, yuv420p_to_rgb24, yuv420p_to_rgba32,
};

/// A single plane of pixel data within a decoded frame.
///
/// For planar YUV formats each plane corresponds to one of Y, U (Cb), or
/// V (Cr).  For packed formats (RGB24, RGBA32) there is a single plane
/// containing all components interleaved.  For NV12 there are two planes:
/// Y and interleaved UV.
#[derive(Debug, Clone)]
pub struct FramePlane {
    /// Raw sample data for this plane.
    data: Vec<u8>,
    /// Width of this plane in samples (pixels).
    width: u32,
    /// Height of this plane in samples (rows).
    height: u32,
    /// Row stride in bytes (may be larger than `width * bytes_per_sample` if
    /// the plane is padded for alignment).
    stride: u32,
}

impl FramePlane {
    /// Create a new plane with the given dimensions and data.
    ///
    /// The stride is set equal to `width * bytes_per_sample` (i.e. tightly
    /// packed) unless the caller explicitly provides padding via
    /// [`FramePlane::with_stride`].
    pub fn new(data: Vec<u8>, width: u32, height: u32) -> Self {
        let stride = width;
        Self {
            data,
            width,
            height,
            stride,
        }
    }

    /// Create a new plane with an explicit row stride.
    pub fn with_stride(data: Vec<u8>, width: u32, height: u32, stride: u32) -> Self {
        Self {
            data,
            width,
            height,
            stride,
        }
    }

    /// Raw sample bytes for this plane.
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Mutable access to the raw sample bytes.
    #[inline]
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Width of this plane in samples.
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height of this plane in rows.
    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Row stride in bytes.
    #[inline]
    pub fn stride(&self) -> u32 {
        self.stride
    }

    /// Return the byte slice for a single row of this plane.
    ///
    /// # Panics
    ///
    /// Panics if `row >= self.height()`.
    #[inline]
    pub fn row(&self, row: u32) -> &[u8] {
        assert!(
            row < self.height,
            "row {row} out of range (height {})",
            self.height
        );
        let start = row as usize * self.stride as usize;
        let end = start + self.width as usize;
        &self.data[start..end]
    }
}

/// Decoded picture type, derived from the H.264 slice type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PictureType {
    /// Intra-coded picture (I-frame).  Decoded without reference to other
    /// pictures.
    I,
    /// Predictive-coded picture (P-frame).  Uses forward prediction from
    /// previously decoded reference pictures.
    P,
    /// Bi-predictive-coded picture (B-frame).  Uses both forward and backward
    /// prediction.
    B,
}

impl std::fmt::Display for PictureType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PictureType::I => f.write_str("I"),
            PictureType::P => f.write_str("P"),
            PictureType::B => f.write_str("B"),
        }
    }
}

/// A fully decoded video frame produced by the [`Decoder`](crate::Decoder).
///
/// The frame stores its pixel data in one or more [`FramePlane`]s whose layout
/// depends on the [`PixelFormat`].  The most common case is YUV 4:2:0 planar
/// (three planes: Y, U, V) which is the native output of the H.264 decoder
/// and the format expected by most downstream video encoders.
///
/// # Converting between pixel formats
///
/// If you need RGB output you can either configure the decoder to produce it
/// directly (which adds a per-frame conversion cost) or call
/// [`DecodedFrame::to_rgb24`] / [`DecodedFrame::to_rgba32`] on demand.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    /// Per-plane pixel data.
    planes: Vec<FramePlane>,
    /// Coded / display width in luma pixels.
    width: u32,
    /// Coded / display height in luma pixels.
    height: u32,
    /// Pixel format of this frame's data.
    pixel_format: PixelFormat,
    /// Optional presentation timestamp carried through from the input NAL
    /// units.  The decoder does not interpret this value – it is passed
    /// through for the caller's convenience.
    pts: Option<u64>,
    /// Picture type (I / P / B).
    picture_type: Option<PictureType>,
    /// Frame number in decode order (from `frame_num` in the slice header).
    frame_num: u32,
    /// Picture order count – determines display order relative to other
    /// decoded pictures.
    pic_order_cnt: i32,
    /// True if this picture is used as a reference by other pictures.
    is_reference: bool,
    /// True if this picture is an IDR (Instantaneous Decoder Refresh) that
    /// resets the DPB.
    is_idr: bool,
    /// Colour matrix that was used (or should be used) for YUV↔RGB conversion.
    colour_matrix: ColourMatrix,
}

impl DecodedFrame {
    // ------------------------------------------------------------------
    // Construction (crate-internal)
    // ------------------------------------------------------------------

    /// Create a new decoded frame from YUV 4:2:0 planar data.
    ///
    /// This is the primary constructor used by the decoder core.
    pub(crate) fn from_yuv420p(
        y_plane: Vec<u8>,
        u_plane: Vec<u8>,
        v_plane: Vec<u8>,
        width: u32,
        height: u32,
    ) -> Self {
        let chroma_w = width.div_ceil(2);
        let chroma_h = height.div_ceil(2);

        let planes = vec![
            FramePlane::new(y_plane, width, height),
            FramePlane::new(u_plane, chroma_w, chroma_h),
            FramePlane::new(v_plane, chroma_w, chroma_h),
        ];

        Self {
            planes,
            width,
            height,
            pixel_format: PixelFormat::Yuv420p,
            pts: None,
            picture_type: None,
            frame_num: 0,
            pic_order_cnt: 0,
            is_reference: false,
            is_idr: false,
            colour_matrix: ColourMatrix::default(),
        }
    }

    /// Create a frame from pre-built planes and explicit format.
    pub(crate) fn from_planes(
        planes: Vec<FramePlane>,
        width: u32,
        height: u32,
        pixel_format: PixelFormat,
    ) -> Self {
        Self {
            planes,
            width,
            height,
            pixel_format,
            pts: None,
            picture_type: None,
            frame_num: 0,
            pic_order_cnt: 0,
            is_reference: false,
            is_idr: false,
            colour_matrix: ColourMatrix::default(),
        }
    }

    // ------------------------------------------------------------------
    // Builder-style setters (crate-internal)
    // ------------------------------------------------------------------

    pub(crate) fn with_pts(mut self, pts: u64) -> Self {
        self.pts = Some(pts);
        self
    }

    pub(crate) fn with_picture_type(mut self, pt: PictureType) -> Self {
        self.picture_type = Some(pt);
        self
    }

    pub(crate) fn with_frame_num(mut self, frame_num: u32) -> Self {
        self.frame_num = frame_num;
        self
    }

    pub(crate) fn with_pic_order_cnt(mut self, poc: i32) -> Self {
        self.pic_order_cnt = poc;
        self
    }

    pub(crate) fn with_is_reference(mut self, is_ref: bool) -> Self {
        self.is_reference = is_ref;
        self
    }

    pub(crate) fn with_is_idr(mut self, is_idr: bool) -> Self {
        self.is_idr = is_idr;
        self
    }

    pub(crate) fn with_colour_matrix(mut self, matrix: ColourMatrix) -> Self {
        self.colour_matrix = matrix;
        self
    }

    // ------------------------------------------------------------------
    // Public accessors
    // ------------------------------------------------------------------

    /// Frame width in luma pixels.
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Frame height in luma pixels.
    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Pixel format of the frame data.
    #[inline]
    pub fn pixel_format(&self) -> PixelFormat {
        self.pixel_format
    }

    /// Presentation timestamp (if set).
    #[inline]
    pub fn pts(&self) -> Option<u64> {
        self.pts
    }

    /// Picture type (I / P / B), if known.
    #[inline]
    pub fn picture_type(&self) -> Option<PictureType> {
        self.picture_type
    }

    /// Frame number in decode order.
    #[inline]
    pub fn frame_num(&self) -> u32 {
        self.frame_num
    }

    /// Picture order count (display order).
    #[inline]
    pub fn pic_order_cnt(&self) -> i32 {
        self.pic_order_cnt
    }

    /// Whether this picture is a reference frame.
    #[inline]
    pub fn is_reference(&self) -> bool {
        self.is_reference
    }

    /// Whether this picture is an IDR frame.
    #[inline]
    pub fn is_idr(&self) -> bool {
        self.is_idr
    }

    /// The colour matrix used for this frame.
    #[inline]
    pub fn colour_matrix(&self) -> ColourMatrix {
        self.colour_matrix
    }

    /// Number of planes in this frame.
    #[inline]
    pub fn num_planes(&self) -> usize {
        self.planes.len()
    }

    /// Access a single plane by index.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.num_planes()`.
    #[inline]
    pub fn plane(&self, index: usize) -> &FramePlane {
        &self.planes[index]
    }

    /// Mutable access to a single plane by index.
    #[inline]
    pub fn plane_mut(&mut self, index: usize) -> &mut FramePlane {
        &mut self.planes[index]
    }

    /// Slice of all planes.
    #[inline]
    pub fn planes(&self) -> &[FramePlane] {
        &self.planes
    }

    /// All raw pixel data concatenated across planes.
    ///
    /// For planar formats this returns Y followed by U followed by V.  For
    /// packed formats this returns the single packed buffer.
    pub fn data(&self) -> Vec<u8> {
        let total: usize = self.planes.iter().map(|p| p.data.len()).sum();
        let mut buf = Vec::with_capacity(total);
        for plane in &self.planes {
            buf.extend_from_slice(&plane.data);
        }
        buf
    }

    // ------------------------------------------------------------------
    // Convenience: Y / U / V accessors for planar YUV formats
    // ------------------------------------------------------------------

    /// The luma (Y) plane.
    ///
    /// # Panics
    ///
    /// Panics if the frame is not in a planar YUV or NV12 format.
    #[inline]
    pub fn y_plane(&self) -> &FramePlane {
        assert!(
            self.pixel_format.is_planar_yuv() || self.pixel_format == PixelFormat::Nv12,
            "y_plane() called on non-YUV frame (format: {})",
            self.pixel_format,
        );
        &self.planes[0]
    }

    /// The Cb (U) chroma plane.
    ///
    /// # Panics
    ///
    /// Panics if the frame is not in a planar YUV format.
    #[inline]
    pub fn u_plane(&self) -> &FramePlane {
        assert!(
            self.pixel_format.is_planar_yuv(),
            "u_plane() called on non-planar-YUV frame (format: {})",
            self.pixel_format,
        );
        &self.planes[1]
    }

    /// The Cr (V) chroma plane.
    ///
    /// # Panics
    ///
    /// Panics if the frame is not in a planar YUV format.
    #[inline]
    pub fn v_plane(&self) -> &FramePlane {
        assert!(
            self.pixel_format.is_planar_yuv(),
            "v_plane() called on non-planar-YUV frame (format: {})",
            self.pixel_format,
        );
        &self.planes[2]
    }

    // ------------------------------------------------------------------
    // Format conversion helpers
    // ------------------------------------------------------------------

    /// Convert this frame to packed RGB24.
    ///
    /// If the frame is already in RGB24 format, a clone of the data is
    /// returned.  Otherwise the frame must be in YUV 4:2:0 planar format
    /// and the BT.601/BT.709 conversion (as configured) is applied.
    ///
    /// # Panics
    ///
    /// Panics if the source format is not YUV 4:2:0 planar or RGB24.
    pub fn to_rgb24(&self) -> DecodedFrame {
        if self.pixel_format == PixelFormat::Rgb24 {
            return self.clone();
        }
        assert_eq!(
            self.pixel_format,
            PixelFormat::Yuv420p,
            "to_rgb24() currently only supports YUV 4:2:0 planar input, got {}",
            self.pixel_format,
        );

        let rgb = yuv420p_to_rgb24(
            self.planes[0].data(),
            self.planes[1].data(),
            self.planes[2].data(),
            self.width,
            self.height,
            self.colour_matrix,
        );

        let plane = FramePlane::with_stride(rgb, self.width * 3, self.height, self.width * 3);

        let mut frame =
            DecodedFrame::from_planes(vec![plane], self.width, self.height, PixelFormat::Rgb24);
        frame.pts = self.pts;
        frame.picture_type = self.picture_type;
        frame.frame_num = self.frame_num;
        frame.pic_order_cnt = self.pic_order_cnt;
        frame.is_reference = self.is_reference;
        frame.is_idr = self.is_idr;
        frame.colour_matrix = self.colour_matrix;
        frame
    }

    /// Convert this frame to packed RGBA32.
    ///
    /// See [`DecodedFrame::to_rgb24`] for details – this variant adds a
    /// fully-opaque alpha channel.
    ///
    /// # Panics
    ///
    /// Panics if the source format is not YUV 4:2:0 planar or RGBA32.
    pub fn to_rgba32(&self) -> DecodedFrame {
        if self.pixel_format == PixelFormat::Rgba32 {
            return self.clone();
        }
        assert_eq!(
            self.pixel_format,
            PixelFormat::Yuv420p,
            "to_rgba32() currently only supports YUV 4:2:0 planar input, got {}",
            self.pixel_format,
        );

        let rgba = yuv420p_to_rgba32(
            self.planes[0].data(),
            self.planes[1].data(),
            self.planes[2].data(),
            self.width,
            self.height,
            self.colour_matrix,
        );

        let plane = FramePlane::with_stride(rgba, self.width * 4, self.height, self.width * 4);

        let mut frame =
            DecodedFrame::from_planes(vec![plane], self.width, self.height, PixelFormat::Rgba32);
        frame.pts = self.pts;
        frame.picture_type = self.picture_type;
        frame.frame_num = self.frame_num;
        frame.pic_order_cnt = self.pic_order_cnt;
        frame.is_reference = self.is_reference;
        frame.is_idr = self.is_idr;
        frame.colour_matrix = self.colour_matrix;
        frame
    }

    /// Convert this frame to NV12 semi-planar format.
    ///
    /// # Panics
    ///
    /// Panics if the source format is not YUV 4:2:0 planar or NV12.
    pub fn to_nv12(&self) -> DecodedFrame {
        if self.pixel_format == PixelFormat::Nv12 {
            return self.clone();
        }
        assert_eq!(
            self.pixel_format,
            PixelFormat::Yuv420p,
            "to_nv12() currently only supports YUV 4:2:0 planar input, got {}",
            self.pixel_format,
        );

        let nv12 = yuv420p_to_nv12(
            self.planes[0].data(),
            self.planes[1].data(),
            self.planes[2].data(),
            self.width,
            self.height,
        );

        let chroma_h = self.height.div_ceil(2);
        let y_size = (self.width as usize) * (self.height as usize);

        let y_plane = FramePlane::new(nv12[..y_size].to_vec(), self.width, self.height);
        let uv_plane =
            FramePlane::with_stride(nv12[y_size..].to_vec(), self.width, chroma_h, self.width);

        let mut frame = DecodedFrame::from_planes(
            vec![y_plane, uv_plane],
            self.width,
            self.height,
            PixelFormat::Nv12,
        );
        frame.pts = self.pts;
        frame.picture_type = self.picture_type;
        frame.frame_num = self.frame_num;
        frame.pic_order_cnt = self.pic_order_cnt;
        frame.is_reference = self.is_reference;
        frame.is_idr = self.is_idr;
        frame.colour_matrix = self.colour_matrix;
        frame
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_frame(width: u32, height: u32) -> DecodedFrame {
        let luma_size = (width * height) as usize;
        let chroma_w = ((width + 1) / 2) as usize;
        let chroma_h = ((height + 1) / 2) as usize;
        let chroma_size = chroma_w * chroma_h;

        // Neutral grey: Y=128, Cb=128, Cr=128
        let y = vec![128u8; luma_size];
        let u = vec![128u8; chroma_size];
        let v = vec![128u8; chroma_size];

        DecodedFrame::from_yuv420p(y, u, v, width, height)
            .with_frame_num(42)
            .with_pic_order_cnt(7)
            .with_is_reference(true)
            .with_is_idr(true)
            .with_picture_type(PictureType::I)
    }

    #[test]
    fn test_basic_accessors() {
        let frame = make_test_frame(320, 240);

        assert_eq!(frame.width(), 320);
        assert_eq!(frame.height(), 240);
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420p);
        assert_eq!(frame.num_planes(), 3);
        assert_eq!(frame.frame_num(), 42);
        assert_eq!(frame.pic_order_cnt(), 7);
        assert!(frame.is_reference());
        assert!(frame.is_idr());
        assert_eq!(frame.picture_type(), Some(PictureType::I));
        assert_eq!(frame.pts(), None);
    }

    #[test]
    fn test_with_pts() {
        let frame = make_test_frame(16, 16).with_pts(12345);
        assert_eq!(frame.pts(), Some(12345));
    }

    #[test]
    fn test_plane_dimensions() {
        let frame = make_test_frame(320, 240);

        assert_eq!(frame.y_plane().width(), 320);
        assert_eq!(frame.y_plane().height(), 240);

        assert_eq!(frame.u_plane().width(), 160);
        assert_eq!(frame.u_plane().height(), 120);

        assert_eq!(frame.v_plane().width(), 160);
        assert_eq!(frame.v_plane().height(), 120);
    }

    #[test]
    fn test_plane_odd_dimensions() {
        let frame = make_test_frame(3, 3);

        assert_eq!(frame.y_plane().width(), 3);
        assert_eq!(frame.y_plane().height(), 3);
        assert_eq!(frame.y_plane().data().len(), 9);

        // (3+1)/2 = 2
        assert_eq!(frame.u_plane().width(), 2);
        assert_eq!(frame.u_plane().height(), 2);
        assert_eq!(frame.u_plane().data().len(), 4);
    }

    #[test]
    fn test_data_concatenation() {
        let frame = make_test_frame(4, 4);
        let data = frame.data();
        // Y: 4*4 = 16, U: 2*2 = 4, V: 2*2 = 4 => total 24
        assert_eq!(data.len(), 24);
    }

    #[test]
    fn test_row_accessor() {
        let mut y = vec![0u8; 8];
        // Row 0: [1, 2, 3, 4], Row 1: [5, 6, 7, 8]
        for (i, val) in y.iter_mut().enumerate() {
            *val = (i + 1) as u8;
        }
        let u = vec![128u8; 2];
        let v = vec![128u8; 2];
        let frame = DecodedFrame::from_yuv420p(y, u, v, 4, 2);
        assert_eq!(frame.y_plane().row(0), &[1, 2, 3, 4]);
        assert_eq!(frame.y_plane().row(1), &[5, 6, 7, 8]);
    }

    #[test]
    #[should_panic(expected = "row 2 out of range")]
    fn test_row_out_of_bounds() {
        let frame = make_test_frame(4, 2);
        let _ = frame.y_plane().row(2);
    }

    #[test]
    fn test_to_rgb24() {
        let frame = make_test_frame(4, 4).with_pts(999).with_frame_num(7);
        let rgb = frame.to_rgb24();

        assert_eq!(rgb.pixel_format(), PixelFormat::Rgb24);
        assert_eq!(rgb.width(), 4);
        assert_eq!(rgb.height(), 4);
        assert_eq!(rgb.num_planes(), 1);
        assert_eq!(rgb.data().len(), 4 * 4 * 3);
        // Metadata is preserved
        assert_eq!(rgb.pts(), Some(999));
        assert_eq!(rgb.frame_num(), 7);
    }

    #[test]
    fn test_to_rgb24_idempotent() {
        let frame = make_test_frame(4, 4);
        let rgb = frame.to_rgb24();
        let rgb2 = rgb.to_rgb24();
        assert_eq!(rgb.data(), rgb2.data());
    }

    #[test]
    fn test_to_rgba32() {
        let frame = make_test_frame(4, 4);
        let rgba = frame.to_rgba32();

        assert_eq!(rgba.pixel_format(), PixelFormat::Rgba32);
        assert_eq!(rgba.data().len(), 4 * 4 * 4);

        // Every 4th byte (alpha) should be 0xFF
        let data = rgba.data();
        for pixel in data.chunks(4) {
            assert_eq!(pixel[3], 0xFF);
        }
    }

    #[test]
    fn test_to_nv12() {
        let frame = make_test_frame(4, 4).with_pts(42);
        let nv12 = frame.to_nv12();

        assert_eq!(nv12.pixel_format(), PixelFormat::Nv12);
        assert_eq!(nv12.width(), 4);
        assert_eq!(nv12.height(), 4);
        assert_eq!(nv12.num_planes(), 2);
        assert_eq!(nv12.pts(), Some(42));

        // Y plane: 4*4 = 16 bytes
        assert_eq!(nv12.plane(0).data().len(), 16);
        // UV plane: 4*2 = 8 bytes (width * chroma_height)
        assert_eq!(nv12.plane(1).data().len(), 8);
    }

    #[test]
    fn test_picture_type_display() {
        assert_eq!(format!("{}", PictureType::I), "I");
        assert_eq!(format!("{}", PictureType::P), "P");
        assert_eq!(format!("{}", PictureType::B), "B");
    }
}

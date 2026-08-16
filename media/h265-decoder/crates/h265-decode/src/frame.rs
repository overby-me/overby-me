//! Decoded frame representation for the H.265/HEVC decoder.
//!
//! A [`DecodedFrame`] holds the reconstructed picture data (typically YUV
//! 4:2:0 planar) together with metadata such as picture order count, picture
//! type, and reference status.  Individual colour planes are accessible via
//! [`FramePlane`] references.

use crate::pixel::{
    ColourMatrix, PixelFormat, yuv420p_to_nv12, yuv420p_to_rgb24, yuv420p_to_rgba32,
};

/// A single colour plane of a decoded picture.
///
/// Each plane stores its sample data contiguously in row-major order.  The
/// `stride` may be larger than `width` when the plane has padding (e.g. for
/// alignment to CTU boundaries).
#[derive(Debug, Clone)]
pub struct FramePlane {
    /// Raw sample data.
    data: Vec<u8>,
    /// Width of the plane in samples.
    width: u32,
    /// Height of the plane in rows.
    height: u32,
    /// Number of bytes per row (≥ width).  For densely packed planes this
    /// equals `width`.
    stride: u32,
}

impl FramePlane {
    /// Create a new plane with `stride == width` (densely packed).
    ///
    /// # Panics
    ///
    /// Panics if `data.len() < width * height`.
    pub fn new(data: Vec<u8>, width: u32, height: u32) -> Self {
        assert!(
            data.len() >= (width as usize) * (height as usize),
            "plane data too short: need {} bytes, got {}",
            width as usize * height as usize,
            data.len(),
        );
        Self {
            data,
            width,
            height,
            stride: width,
        }
    }

    /// Create a new plane with an explicit stride.
    pub fn with_stride(data: Vec<u8>, width: u32, height: u32, stride: u32) -> Self {
        assert!(stride >= width);
        assert!(data.len() >= (stride as usize) * (height as usize));
        Self {
            data,
            width,
            height,
            stride,
        }
    }

    /// Raw sample data.
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Mutable access to the raw sample data.
    #[inline]
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Width of the plane in samples.
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height of the plane in rows.
    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Stride (bytes per row).
    #[inline]
    pub fn stride(&self) -> u32 {
        self.stride
    }

    /// Return the sample data for a single row.
    ///
    /// # Panics
    ///
    /// Panics if `row >= self.height()`.
    pub fn row(&self, row: u32) -> &[u8] {
        assert!(
            row < self.height,
            "row {row} out of bounds (height={})",
            self.height
        );
        let start = (row as usize) * (self.stride as usize);
        let end = start + (self.width as usize);
        &self.data[start..end]
    }
}

// ---------------------------------------------------------------------------
// PictureType
// ---------------------------------------------------------------------------

/// Type of a decoded picture (I, P, or B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PictureType {
    /// Intra-predicted picture (no references).
    I,
    /// Uni-directionally predicted picture (references from list 0).
    P,
    /// Bi-directionally predicted picture (references from lists 0 and 1).
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

// ---------------------------------------------------------------------------
// DecodedFrame
// ---------------------------------------------------------------------------

/// A fully decoded video frame with metadata.
///
/// The frame stores its pixel data as a vector of [`FramePlane`]s.  For the
/// common YUV 4:2:0 case there are three planes (Y, U/Cb, V/Cr).  Packed
/// formats like RGB24 use a single plane.
///
/// # Metadata
///
/// In addition to the pixel data, each frame carries:
///
/// * **picture order count** (`pic_order_cnt`) – the display order index.
/// * **picture type** – I, P, or B.
/// * **frame number** – decode-order index within the coded video sequence.
/// * **reference status** – whether this frame is used as a reference.
/// * **IRAP flag** – whether this picture is an Intra Random Access Point.
/// * **colour matrix** – the YUV→RGB conversion matrix in use.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    /// Colour planes.
    planes: Vec<FramePlane>,
    /// Picture width in luma samples.
    width: u32,
    /// Picture height in luma samples.
    height: u32,
    /// Pixel format of this frame.
    pixel_format: PixelFormat,
    /// Presentation timestamp (optional, set by the caller / container).
    pts: Option<u64>,
    /// Picture type (I / P / B).
    picture_type: PictureType,
    /// Frame number in decode order.
    frame_num: u32,
    /// Picture order count (display order).
    pic_order_cnt: i32,
    /// Whether this frame is used as a reference for other pictures.
    is_reference: bool,
    /// Whether this picture is an Intra Random Access Point (IDR, CRA,
    /// BLA, etc.).
    is_irap: bool,
    /// Colour matrix for YUV→RGB conversion.
    colour_matrix: ColourMatrix,
}

impl DecodedFrame {
    // ------------------------------------------------------------------
    // Construction
    // ------------------------------------------------------------------

    /// Create a frame from pre-separated YUV 4:2:0 planes.
    ///
    /// `y`, `u`, `v` are the raw plane buffers.  The chroma planes must
    /// each have `ceil(width/2) * ceil(height/2)` samples.
    pub(crate) fn from_yuv420p(
        y: Vec<u8>,
        u: Vec<u8>,
        v: Vec<u8>,
        width: u32,
        height: u32,
    ) -> Self {
        let chroma_w = width.div_ceil(2);
        let chroma_h = height.div_ceil(2);

        let planes = vec![
            FramePlane::new(y, width, height),
            FramePlane::new(u, chroma_w, chroma_h),
            FramePlane::new(v, chroma_w, chroma_h),
        ];

        Self {
            planes,
            width,
            height,
            pixel_format: PixelFormat::Yuv420p,
            pts: None,
            picture_type: PictureType::I,
            frame_num: 0,
            pic_order_cnt: 0,
            is_reference: false,
            is_irap: false,
            colour_matrix: ColourMatrix::default(),
        }
    }

    /// Create a frame from a vector of pre-built planes.
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
            picture_type: PictureType::I,
            frame_num: 0,
            pic_order_cnt: 0,
            is_reference: false,
            is_irap: false,
            colour_matrix: ColourMatrix::default(),
        }
    }

    // ------------------------------------------------------------------
    // Builder-style setters (crate-internal)
    // ------------------------------------------------------------------

    /// Set the presentation timestamp.
    pub(crate) fn with_pts(mut self, pts: u64) -> Self {
        self.pts = Some(pts);
        self
    }

    pub(crate) fn with_picture_type(mut self, pt: PictureType) -> Self {
        self.picture_type = pt;
        self
    }

    pub(crate) fn with_frame_num(mut self, n: u32) -> Self {
        self.frame_num = n;
        self
    }

    pub(crate) fn with_pic_order_cnt(mut self, poc: i32) -> Self {
        self.pic_order_cnt = poc;
        self
    }

    pub(crate) fn with_is_reference(mut self, r: bool) -> Self {
        self.is_reference = r;
        self
    }

    pub(crate) fn with_is_irap(mut self, irap: bool) -> Self {
        self.is_irap = irap;
        self
    }

    pub(crate) fn with_colour_matrix(mut self, cm: ColourMatrix) -> Self {
        self.colour_matrix = cm;
        self
    }

    // ------------------------------------------------------------------
    // Public accessors
    // ------------------------------------------------------------------

    /// Picture width in luma samples.
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Picture height in luma samples.
    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Pixel format of this frame.
    #[inline]
    pub fn pixel_format(&self) -> PixelFormat {
        self.pixel_format
    }

    /// Optional presentation timestamp.
    #[inline]
    pub fn pts(&self) -> Option<u64> {
        self.pts
    }

    /// Picture type (I / P / B).
    #[inline]
    pub fn picture_type(&self) -> PictureType {
        self.picture_type
    }

    /// Frame number (decode order).
    #[inline]
    pub fn frame_num(&self) -> u32 {
        self.frame_num
    }

    /// Picture order count (display order).
    #[inline]
    pub fn pic_order_cnt(&self) -> i32 {
        self.pic_order_cnt
    }

    /// Whether this frame is used as a reference.
    #[inline]
    pub fn is_reference(&self) -> bool {
        self.is_reference
    }

    /// Whether this picture is an Intra Random Access Point.
    #[inline]
    pub fn is_irap(&self) -> bool {
        self.is_irap
    }

    /// Colour matrix for YUV→RGB conversion.
    #[inline]
    pub fn colour_matrix(&self) -> ColourMatrix {
        self.colour_matrix
    }

    /// Number of planes in this frame.
    #[inline]
    pub fn num_planes(&self) -> usize {
        self.planes.len()
    }

    /// Access a plane by index.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.num_planes()`.
    pub fn plane(&self, index: usize) -> &FramePlane {
        &self.planes[index]
    }

    /// Mutable access to a plane by index.
    pub fn plane_mut(&mut self, index: usize) -> &mut FramePlane {
        &mut self.planes[index]
    }

    /// All planes as a slice.
    pub fn planes(&self) -> &[FramePlane] {
        &self.planes
    }

    /// Concatenated raw data from all planes (useful for simple I/O).
    pub fn data(&self) -> Vec<u8> {
        let total: usize = self.planes.iter().map(|p| p.data().len()).sum();
        let mut buf = Vec::with_capacity(total);
        for p in &self.planes {
            buf.extend_from_slice(p.data());
        }
        buf
    }

    // ------------------------------------------------------------------
    // Convenience plane accessors for YUV 4:2:0
    // ------------------------------------------------------------------

    /// Y (luma) plane.
    ///
    /// # Panics
    ///
    /// Panics if the frame has fewer than one plane or is not a planar YUV
    /// format.
    pub fn y_plane(&self) -> &FramePlane {
        assert!(
            self.pixel_format.is_planar_yuv(),
            "y_plane() requires a planar YUV format, got {:?}",
            self.pixel_format,
        );
        &self.planes[0]
    }

    /// U (Cb) chroma plane.
    ///
    /// # Panics
    ///
    /// Panics if the frame has fewer than two planes.
    pub fn u_plane(&self) -> &FramePlane {
        assert!(
            self.pixel_format.is_planar_yuv(),
            "u_plane() requires a planar YUV format, got {:?}",
            self.pixel_format,
        );
        &self.planes[1]
    }

    /// V (Cr) chroma plane.
    ///
    /// # Panics
    ///
    /// Panics if the frame has fewer than three planes.
    pub fn v_plane(&self) -> &FramePlane {
        assert!(
            self.pixel_format.is_planar_yuv(),
            "v_plane() requires a planar YUV format, got {:?}",
            self.pixel_format,
        );
        &self.planes[2]
    }

    // ------------------------------------------------------------------
    // Format conversions
    // ------------------------------------------------------------------

    /// Convert this frame to packed RGB24.
    ///
    /// If the frame is already RGB24 it is returned unchanged.  Otherwise
    /// the YUV 4:2:0 data is converted using the frame's colour matrix.
    pub fn to_rgb24(&self) -> DecodedFrame {
        if self.pixel_format == PixelFormat::Rgb24 {
            return self.clone();
        }

        let rgb = yuv420p_to_rgb24(
            self.planes[0].data(),
            self.planes[1].data(),
            self.planes[2].data(),
            self.width,
            self.height,
            self.colour_matrix,
        );

        let plane = FramePlane::new(rgb, self.width * 3, self.height);

        let mut f =
            DecodedFrame::from_planes(vec![plane], self.width, self.height, PixelFormat::Rgb24);
        f.pts = self.pts;
        f.picture_type = self.picture_type;
        f.frame_num = self.frame_num;
        f.pic_order_cnt = self.pic_order_cnt;
        f.is_reference = self.is_reference;
        f.is_irap = self.is_irap;
        f.colour_matrix = self.colour_matrix;
        f
    }

    /// Convert this frame to packed RGBA32.
    pub fn to_rgba32(&self) -> DecodedFrame {
        if self.pixel_format == PixelFormat::Rgba32 {
            return self.clone();
        }

        let rgba = yuv420p_to_rgba32(
            self.planes[0].data(),
            self.planes[1].data(),
            self.planes[2].data(),
            self.width,
            self.height,
            self.colour_matrix,
        );

        let plane = FramePlane::new(rgba, self.width * 4, self.height);

        let mut f =
            DecodedFrame::from_planes(vec![plane], self.width, self.height, PixelFormat::Rgba32);
        f.pts = self.pts;
        f.picture_type = self.picture_type;
        f.frame_num = self.frame_num;
        f.pic_order_cnt = self.pic_order_cnt;
        f.is_reference = self.is_reference;
        f.is_irap = self.is_irap;
        f.colour_matrix = self.colour_matrix;
        f
    }

    /// Convert this frame to NV12 semi-planar.
    pub fn to_nv12(&self) -> DecodedFrame {
        if self.pixel_format == PixelFormat::Nv12 {
            return self.clone();
        }

        let nv12 = yuv420p_to_nv12(
            self.planes[0].data(),
            self.planes[1].data(),
            self.planes[2].data(),
            self.width,
            self.height,
        );

        let y_size = (self.width as usize) * (self.height as usize);
        let y_plane = FramePlane::new(nv12[..y_size].to_vec(), self.width, self.height);
        let chroma_h = self.height.div_ceil(2);
        let uv_plane = FramePlane::new(nv12[y_size..].to_vec(), self.width, chroma_h);

        let mut f = DecodedFrame::from_planes(
            vec![y_plane, uv_plane],
            self.width,
            self.height,
            PixelFormat::Nv12,
        );
        f.pts = self.pts;
        f.picture_type = self.picture_type;
        f.frame_num = self.frame_num;
        f.pic_order_cnt = self.pic_order_cnt;
        f.is_reference = self.is_reference;
        f.is_irap = self.is_irap;
        f.colour_matrix = self.colour_matrix;
        f
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
        let chroma_w = width.div_ceil(2) as usize;
        let chroma_h = height.div_ceil(2) as usize;
        let chroma_size = chroma_w * chroma_h;

        let y = vec![128u8; luma_size];
        let u = vec![128u8; chroma_size];
        let v = vec![128u8; chroma_size];

        DecodedFrame::from_yuv420p(y, u, v, width, height)
    }

    #[test]
    fn test_basic_accessors() {
        let frame = make_test_frame(320, 240);
        assert_eq!(frame.width(), 320);
        assert_eq!(frame.height(), 240);
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420p);
        assert_eq!(frame.num_planes(), 3);
        assert_eq!(frame.picture_type(), PictureType::I);
        assert_eq!(frame.frame_num(), 0);
        assert_eq!(frame.pic_order_cnt(), 0);
        assert!(!frame.is_reference());
        assert!(!frame.is_irap());
        assert_eq!(frame.pts(), None);
    }

    #[test]
    fn test_with_pts() {
        let frame = make_test_frame(16, 16).with_pts(42);
        assert_eq!(frame.pts(), Some(42));
    }

    #[test]
    fn test_plane_dimensions() {
        let frame = make_test_frame(320, 240);
        let y = frame.y_plane();
        assert_eq!(y.width(), 320);
        assert_eq!(y.height(), 240);
        assert_eq!(y.stride(), 320);

        let u = frame.u_plane();
        assert_eq!(u.width(), 160);
        assert_eq!(u.height(), 120);

        let v = frame.v_plane();
        assert_eq!(v.width(), 160);
        assert_eq!(v.height(), 120);
    }

    #[test]
    fn test_plane_odd_dimensions() {
        let frame = make_test_frame(3, 3);
        let y = frame.y_plane();
        assert_eq!(y.width(), 3);
        assert_eq!(y.height(), 3);

        let u = frame.u_plane();
        assert_eq!(u.width(), 2);
        assert_eq!(u.height(), 2);
    }

    #[test]
    fn test_data_concatenation() {
        let frame = make_test_frame(4, 4);
        let data = frame.data();
        // Y=16, U=4, V=4 => total 24
        assert_eq!(data.len(), 4 * 4 + 2 * 2 + 2 * 2);
    }

    #[test]
    fn test_row_accessor() {
        let y = vec![1, 2, 3, 4, 5, 6];
        let u = vec![10u8; 2];
        let v = vec![20u8; 2];
        let frame = DecodedFrame::from_yuv420p(y, u, v, 3, 2);

        let row0 = frame.y_plane().row(0);
        assert_eq!(row0, &[1, 2, 3]);

        let row1 = frame.y_plane().row(1);
        assert_eq!(row1, &[4, 5, 6]);
    }

    #[test]
    #[should_panic(expected = "row 2 out of bounds")]
    fn test_row_out_of_bounds() {
        let frame = make_test_frame(4, 2);
        frame.y_plane().row(2);
    }

    #[test]
    fn test_to_rgb24() {
        let frame = make_test_frame(2, 2);
        let rgb = frame.to_rgb24();
        assert_eq!(rgb.pixel_format(), PixelFormat::Rgb24);
        assert_eq!(rgb.width(), 2);
        assert_eq!(rgb.height(), 2);
        assert_eq!(rgb.num_planes(), 1);
    }

    #[test]
    fn test_to_rgb24_idempotent() {
        let frame = make_test_frame(2, 2);
        let rgb1 = frame.to_rgb24();
        let rgb2 = rgb1.to_rgb24();
        assert_eq!(rgb1.data(), rgb2.data());
    }

    #[test]
    fn test_to_rgba32() {
        let frame = make_test_frame(2, 2);
        let rgba = frame.to_rgba32();
        assert_eq!(rgba.pixel_format(), PixelFormat::Rgba32);
        assert_eq!(rgba.num_planes(), 1);
    }

    #[test]
    fn test_to_nv12() {
        let frame = make_test_frame(4, 4);
        let nv12 = frame.to_nv12();
        assert_eq!(nv12.pixel_format(), PixelFormat::Nv12);
        assert_eq!(nv12.num_planes(), 2);
    }

    #[test]
    fn test_picture_type_display() {
        assert_eq!(PictureType::I.to_string(), "I");
        assert_eq!(PictureType::P.to_string(), "P");
        assert_eq!(PictureType::B.to_string(), "B");
    }

    #[test]
    fn test_with_builders() {
        let frame = make_test_frame(16, 16)
            .with_picture_type(PictureType::B)
            .with_frame_num(7)
            .with_pic_order_cnt(14)
            .with_is_reference(true)
            .with_is_irap(true)
            .with_colour_matrix(ColourMatrix::Bt2020);

        assert_eq!(frame.picture_type(), PictureType::B);
        assert_eq!(frame.frame_num(), 7);
        assert_eq!(frame.pic_order_cnt(), 14);
        assert!(frame.is_reference());
        assert!(frame.is_irap());
        assert_eq!(frame.colour_matrix(), ColourMatrix::Bt2020);
    }

    #[test]
    fn test_frame_plane_with_stride() {
        // 3 columns, 2 rows, stride 4 (1 byte padding per row)
        let data = vec![1, 2, 3, 0, 4, 5, 6, 0];
        let plane = FramePlane::with_stride(data, 3, 2, 4);
        assert_eq!(plane.row(0), &[1, 2, 3]);
        assert_eq!(plane.row(1), &[4, 5, 6]);
        assert_eq!(plane.stride(), 4);
    }

    #[test]
    fn test_plane_mut() {
        let mut frame = make_test_frame(4, 4);
        let plane = frame.plane_mut(0);
        plane.data_mut()[0] = 42;
        assert_eq!(frame.plane(0).data()[0], 42);
    }
}

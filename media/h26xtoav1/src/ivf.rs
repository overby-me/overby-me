//! IVF container writer.
//!
//! IVF is a minimal container format for raw video codec bitstreams.  It is
//! commonly used to wrap VP8, VP9, and AV1 elementary streams and is the
//! default output container for tools such as `rav1e` and `aomenc`.
//!
//! ## Format
//!
//! ```text
//! File header  (32 bytes)
//! ┌────────────────────────────────────────────┐
//! │  signature       4 bytes  "DKIF"           │
//! │  version         2 bytes  0                │
//! │  header_size     2 bytes  32               │
//! │  fourcc          4 bytes  e.g. "AV01"      │
//! │  width           2 bytes                   │
//! │  height          2 bytes                   │
//! │  timebase_num    4 bytes                   │
//! │  timebase_den    4 bytes                   │
//! │  frame_count     4 bytes                   │
//! │  unused          4 bytes  0                │
//! └────────────────────────────────────────────┘
//!
//! Per-frame header (12 bytes) + payload
//! ┌────────────────────────────────────────────┐
//! │  frame_size      4 bytes  (little-endian)  │
//! │  timestamp       8 bytes  (little-endian)  │
//! │  payload         frame_size bytes           │
//! └────────────────────────────────────────────┘
//! ```

use std::io::{self, Seek, SeekFrom, Write};

/// AV1 FourCC bytes: `"AV01"`.
const AV1_FOURCC: &[u8; 4] = b"AV01";

/// IVF file header size in bytes.
const IVF_HEADER_SIZE: u16 = 32;

/// IVF file signature.
const IVF_SIGNATURE: &[u8; 4] = b"DKIF";

/// Writer that produces an IVF container around an AV1 elementary stream.
///
/// The writer keeps track of the number of frames written so that it can
/// update the frame-count field in the file header when [`finish`](IvfWriter::finish)
/// is called.
pub struct IvfWriter<W: Write + Seek> {
    inner: W,
    frame_count: u32,
}

impl<W: Write + Seek> IvfWriter<W> {
    /// Create a new IVF writer and write the 32-byte file header.
    ///
    /// The `timebase_num` / `timebase_den` pair defines the timebase for
    /// frame timestamps.  A common choice is `1 / fps` (e.g. `1 / 30`).
    pub fn new(
        mut writer: W,
        width: u16,
        height: u16,
        timebase_num: u32,
        timebase_den: u32,
    ) -> io::Result<Self> {
        // signature
        writer.write_all(IVF_SIGNATURE)?;
        // version
        writer.write_all(&0u16.to_le_bytes())?;
        // header size
        writer.write_all(&IVF_HEADER_SIZE.to_le_bytes())?;
        // fourcc
        writer.write_all(AV1_FOURCC)?;
        // width, height
        writer.write_all(&width.to_le_bytes())?;
        writer.write_all(&height.to_le_bytes())?;
        // timebase numerator / denominator
        writer.write_all(&timebase_num.to_le_bytes())?;
        writer.write_all(&timebase_den.to_le_bytes())?;
        // frame count (placeholder – updated in finish())
        writer.write_all(&0u32.to_le_bytes())?;
        // unused
        writer.write_all(&0u32.to_le_bytes())?;

        Ok(Self {
            inner: writer,
            frame_count: 0,
        })
    }

    /// Write a single encoded frame (packet) to the IVF file.
    ///
    /// `timestamp` is expressed in the timebase units declared in the header.
    pub fn write_frame(&mut self, data: &[u8], timestamp: u64) -> io::Result<()> {
        let frame_size = data.len() as u32;

        // 12-byte per-frame header
        self.inner.write_all(&frame_size.to_le_bytes())?;
        self.inner.write_all(&timestamp.to_le_bytes())?;

        // payload
        self.inner.write_all(data)?;

        self.frame_count += 1;
        Ok(())
    }

    /// Number of frames written so far.
    #[inline]
    #[allow(dead_code)]
    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    /// Finalise the IVF file by seeking back to the header and writing the
    /// correct frame count, then flush the underlying writer.
    ///
    /// Returns the inner writer.
    pub fn finish(mut self) -> io::Result<W> {
        // Seek to the frame_count field at offset 24.
        self.inner.seek(SeekFrom::Start(24))?;
        self.inner.write_all(&self.frame_count.to_le_bytes())?;

        // Seek back to end so the caller gets the writer in a sensible state.
        self.inner.seek(SeekFrom::End(0))?;
        self.inner.flush()?;

        Ok(self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_header_layout() {
        let buf = Cursor::new(Vec::new());
        let writer = IvfWriter::new(buf, 1920, 1080, 1, 30).unwrap();
        let data = writer.finish().unwrap().into_inner();

        assert_eq!(&data[0..4], b"DKIF");
        assert_eq!(u16::from_le_bytes([data[4], data[5]]), 0); // version
        assert_eq!(u16::from_le_bytes([data[6], data[7]]), 32); // header size
        assert_eq!(&data[8..12], b"AV01");
        assert_eq!(u16::from_le_bytes([data[12], data[13]]), 1920);
        assert_eq!(u16::from_le_bytes([data[14], data[15]]), 1080);
        assert_eq!(
            u32::from_le_bytes([data[16], data[17], data[18], data[19]]),
            1
        );
        assert_eq!(
            u32::from_le_bytes([data[20], data[21], data[22], data[23]]),
            30
        );
        assert_eq!(
            u32::from_le_bytes([data[24], data[25], data[26], data[27]]),
            0
        ); // frame count
        assert_eq!(data.len(), 32);
    }

    #[test]
    fn test_write_frames_and_finish() {
        let buf = Cursor::new(Vec::new());
        let mut writer = IvfWriter::new(buf, 320, 240, 1, 24).unwrap();

        let payload_a = b"frame-one";
        let payload_b = b"frame-two!!";

        writer.write_frame(payload_a, 0).unwrap();
        writer.write_frame(payload_b, 1).unwrap();

        assert_eq!(writer.frame_count(), 2);

        let data = writer.finish().unwrap().into_inner();

        // Frame count in header should now be 2.
        assert_eq!(
            u32::from_le_bytes([data[24], data[25], data[26], data[27]]),
            2
        );

        // First frame header starts at offset 32.
        let off = 32;
        let size_a = u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
        assert_eq!(size_a as usize, payload_a.len());

        let ts_a = u64::from_le_bytes(data[off + 4..off + 12].try_into().unwrap());
        assert_eq!(ts_a, 0);

        assert_eq!(&data[off + 12..off + 12 + payload_a.len()], payload_a);

        // Second frame header follows immediately.
        let off2 = off + 12 + payload_a.len();
        let size_b =
            u32::from_le_bytes([data[off2], data[off2 + 1], data[off2 + 2], data[off2 + 3]]);
        assert_eq!(size_b as usize, payload_b.len());

        let ts_b = u64::from_le_bytes(data[off2 + 4..off2 + 12].try_into().unwrap());
        assert_eq!(ts_b, 1);

        assert_eq!(
            &data[off2 + 12..off2 + 12 + payload_b.len()],
            payload_b.as_slice()
        );
    }
}

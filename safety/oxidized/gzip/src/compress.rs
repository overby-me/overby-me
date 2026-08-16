use flate2::write::DeflateEncoder;
use flate2::{Compression, Crc};
use std::io::{self, Read, Write};

/// Hand-roll the gzip framing so we can control OS=3 (Unix), set the
/// mtime field precisely (source mtime with -N, 0 with -n or from stdin),
/// and avoid `GzBuilder`'s current-time-at-write behavior which breaks
/// the reference/reproducible upstream tests.
pub fn compress_stream<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    level: u32,
    file_name: Option<&str>,
    mtime: u32,
) -> io::Result<()> {
    let mut flags: u8 = 0;
    if file_name.is_some() {
        flags |= 0x08;
    }
    let xfl: u8 = match level {
        9 => 2,
        1 => 4,
        _ => 0,
    };
    writer.write_all(&[0x1f, 0x8b, 0x08, flags])?;
    writer.write_all(&mtime.to_le_bytes())?;
    writer.write_all(&[xfl, 0x03])?;
    if let Some(n) = file_name {
        writer.write_all(n.as_bytes())?;
        writer.write_all(&[0])?;
    }

    let mut crc = Crc::new();
    let mut total: u32 = 0;
    {
        let mut encoder = DeflateEncoder::new(&mut writer, Compression::new(level));
        let mut buf = [0u8; 8192];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            crc.update(&buf[..n]);
            total = total.wrapping_add(n as u32);
            encoder.write_all(&buf[..n])?;
        }
        encoder.finish()?;
    }

    writer.write_all(&crc.sum().to_le_bytes())?;
    writer.write_all(&total.to_le_bytes())?;
    Ok(())
}

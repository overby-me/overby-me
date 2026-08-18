//! # heif-oxide
//!
//! Pure-Rust HEIF/HEIC still-image decoder — no C dependencies, permissively
//! licensed (MIT OR Apache-2.0).
//!
//! Parses the ISOBMFF/HEIF container (ISO/IEC 23008-12) and decodes the HEVC
//! payload with [`rust_h265`], covering what real-world HEIC files use:
//!
//! - **iPhone photos**: grid-tiled primary images (tiles decoded in
//!   parallel), `irot`/`imir`/`clap` orientation, Display P3 → sRGB colour
//!   conversion, 8- and 10-bit HEVC.
//! - Single-picture `hvc1` files from other cameras and encoders.
//!
//! Output is display-ready sRGB, 8-bit (`Rgb8`) or 16-bit (`Rgb16`, for
//! 10/12-bit sources).
//!
//! ```no_run
//! let image = heif_oxide::decode_file("photo.heic").unwrap();
//! println!("{}x{}, {} channels", image.width, image.height, image.channels());
//! let rgba = image.to_rgba8();
//! ```
//!
//! ## Limitations
//!
//! - Decode only — there is no pure-Rust HEVC encoder to pair with.
//! - Alpha auxiliary images are decoded when they are 4:2:0-coded; the more
//!   common monochrome (4:0:0) alpha is skipped (the underlying HEVC
//!   decoder is 4:2:0-only), yielding an image without alpha.
//! - AVIF (`av01` payloads) and JPEG-in-HEIF are rejected with a clear
//!   error, as are protected items and externally-referenced data.
//! - HDR transfer functions (PQ/HLG) are not tone-mapped; wide-gamut
//!   primaries are converted to sRGB assuming an sRGB-like transfer.

mod boxes;
mod bytes;
mod color;
mod error;
mod hevc;
pub(crate) mod par;
#[cfg(test)]
pub(crate) mod test_builder;
mod transform;

use std::path::Path;

use boxes::{ColorProperty, Container, Property};
use color::{RgbBuffer, YuvImage};

pub use color::Nclx;
pub use error::HeifError;

/// Interleaved pixel data. 8-bit sources produce the `*8` variants; 10- and
/// 12-bit sources produce `*16` (samples scaled to the full 0..65535 range).
/// `Rgba*` appears only when the file carries a decodable alpha auxiliary.
pub enum Pixels {
    Rgb8(Vec<u8>),
    Rgba8(Vec<u8>),
    Rgb16(Vec<u16>),
    Rgba16(Vec<u16>),
}

/// What the container declared about colour, for callers that want to know
/// what conversions were applied. Pixels are always delivered as sRGB.
#[derive(Debug, Clone, Default)]
pub struct ColorInfo {
    /// The `colr` nclx description, if present.
    pub nclx: Option<Nclx>,
    /// An ICC profile was present (noted, not applied).
    pub icc_present: bool,
}

/// A decoded HEIF image: display-ready sRGB with orientation applied.
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
    pub color: ColorInfo,
}

impl DecodedImage {
    /// 3 (RGB) or 4 (RGBA).
    pub fn channels(&self) -> u32 {
        match self.pixels {
            Pixels::Rgb8(_) | Pixels::Rgb16(_) => 3,
            Pixels::Rgba8(_) | Pixels::Rgba16(_) => 4,
        }
    }

    /// Bits per sample in the returned buffers (8 or 16).
    pub fn bit_depth(&self) -> u32 {
        match self.pixels {
            Pixels::Rgb8(_) | Pixels::Rgba8(_) => 8,
            Pixels::Rgb16(_) | Pixels::Rgba16(_) => 16,
        }
    }

    /// Interleaved samples normalized to 0.0..=1.0, preserving channel count.
    pub fn to_f32_interleaved(&self) -> Vec<f32> {
        match &self.pixels {
            Pixels::Rgb8(v) | Pixels::Rgba8(v) => v.iter().map(|&s| s as f32 / 255.0).collect(),
            Pixels::Rgb16(v) | Pixels::Rgba16(v) => v.iter().map(|&s| s as f32 / 65535.0).collect(),
        }
    }

    /// Interleaved RGBA, 8 bits per sample (alpha 255 when absent).
    pub fn to_rgba8(&self) -> Vec<u8> {
        let n = (self.width * self.height) as usize;
        let mut out = Vec::with_capacity(n * 4);
        match &self.pixels {
            Pixels::Rgb8(v) => {
                for px in v.chunks_exact(3) {
                    out.extend_from_slice(px);
                    out.push(255);
                }
            }
            Pixels::Rgba8(v) => out.extend_from_slice(v),
            Pixels::Rgb16(v) => {
                for px in v.chunks_exact(3) {
                    out.extend(px.iter().map(|&s| (s >> 8) as u8));
                    out.push(255);
                }
            }
            Pixels::Rgba16(v) => out.extend(v.iter().map(|&s| (s >> 8) as u8)),
        }
        out
    }
}

/// Decode a HEIF/HEIC file from disk.
pub fn decode_file<P: AsRef<Path>>(path: P) -> Result<DecodedImage, HeifError> {
    decode_bytes(&std::fs::read(path)?)
}

/// Decode a HEIF/HEIC file already in memory.
pub fn decode_bytes(data: &[u8]) -> Result<DecodedImage, HeifError> {
    let container = boxes::parse(data)?;
    let primary = container.primary_item;

    // Decode the primary item to a YUV canvas (assembling grid tiles), plus
    // the declared output size the canvas is cropped to.
    let (yuv, out_w, out_h) = decode_item_yuv(&container, primary)?;

    // Colour description: the primary (grid) item's colr wins; tiles may
    // carry their own — used as fallback.
    let color_info = collect_color_info(&container, primary);
    let nclx = color_info.nclx.unwrap_or_default();

    let rgb = color::yuv_to_rgb(&yuv, &nclx);

    // The primary item's transformative properties, in association order.
    let transforms: Vec<Property> = container
        .item_properties(primary)
        .filter(|(_, p)| {
            matches!(
                p,
                Property::Clap { .. } | Property::Irot { .. } | Property::Imir { .. }
            )
        })
        .map(|(_, p)| p.clone())
        .collect();

    // Optional alpha auxiliary image, decoded to a single channel matching
    // the pre-transform geometry. `None` on any failure — alpha is a
    // best-effort enhancement, never a reason to fail the whole file.
    let alpha = decode_alpha_plane(&container, primary, out_w, out_h);

    // Finish per storage width: crop the grid canvas to its output size,
    // interleave alpha, apply orientation transforms.
    let (width, height, pixels) = match rgb {
        RgbBuffer::Rgb8(buf) => {
            let buf = transform::crop_top_left(&buf, yuv.width, 3, out_w, out_h);
            let (buf, has_alpha) = match &alpha {
                Some(a) => (interleave_alpha8(&buf, a), true),
                None => (buf, false),
            };
            let channels = if has_alpha { 4 } else { 3 };
            let (buf, w, h) =
                transform::apply_transforms(buf, out_w, out_h, channels, &transforms)?;
            (
                w,
                h,
                if has_alpha {
                    Pixels::Rgba8(buf)
                } else {
                    Pixels::Rgb8(buf)
                },
            )
        }
        RgbBuffer::Rgb16(buf) => {
            let buf = transform::crop_top_left(&buf, yuv.width, 3, out_w, out_h);
            let (buf, has_alpha) = match &alpha {
                Some(a) => (interleave_alpha16(&buf, a), true),
                None => (buf, false),
            };
            let channels = if has_alpha { 4 } else { 3 };
            let (buf, w, h) =
                transform::apply_transforms(buf, out_w, out_h, channels, &transforms)?;
            (
                w,
                h,
                if has_alpha {
                    Pixels::Rgba16(buf)
                } else {
                    Pixels::Rgb16(buf)
                },
            )
        }
    };

    Ok(DecodedImage {
        width: width as u32,
        height: height as u32,
        pixels,
        color: color_info,
    })
}

/// Reject items that carry an *essential* property we don't understand —
/// the spec forbids processing the item in that case, and silently wrong
/// output (e.g. an unapplied mandatory transform) is worse than an error.
fn check_essential_properties(container: &Container<'_>, item_id: u32) -> Result<(), HeifError> {
    for (essential, property) in container.item_properties(item_id) {
        if essential {
            if let Property::Other(fourcc) = property {
                return Err(HeifError::Unsupported(format!(
                    "essential item property '{}'",
                    bytes::fourcc_str(*fourcc)
                )));
            }
        }
    }
    Ok(())
}

/// Decode any image item — a single `hvc1` picture or a `grid` of them —
/// to a YUV canvas. Returns `(canvas, output_width, output_height)`; for
/// grids the canvas is a whole number of tiles and `output_*` is the
/// (smaller) declared image size.
fn decode_item_yuv(
    container: &Container<'_>,
    item_id: u32,
) -> Result<(YuvImage, usize, usize), HeifError> {
    let info = container
        .items
        .get(&item_id)
        .ok_or_else(|| HeifError::Invalid(format!("item {item_id} not in iinf")))?;
    check_essential_properties(container, item_id)?;
    match &info.item_type {
        b"hvc1" => {
            let yuv = decode_coded_item(container, item_id)?;
            let (w, h) = (yuv.width, yuv.height);
            Ok((yuv, w, h))
        }
        b"grid" => decode_grid(container, item_id),
        other => Err(HeifError::UnsupportedCodec(bytes::fourcc_str(*other))),
    }
}

/// Decode one `hvc1` coded item via rust_h265.
fn decode_coded_item(container: &Container<'_>, item_id: u32) -> Result<YuvImage, HeifError> {
    check_essential_properties(container, item_id)?;
    let hvcc_record = container
        .find_property(item_id, |p| match p {
            Property::HvcC(record) => Some(record.clone()),
            _ => None,
        })
        .ok_or_else(|| HeifError::Invalid(format!("coded item {item_id} has no hvcC property")))?;
    let config = hevc::parse_hvcc(&hvcc_record)?;
    let payload = container.item_payload(item_id)?;
    let annex_b = hevc::build_annex_b(&config, &payload)?;
    let frame = hevc::decode_first_frame(&annex_b)?;
    YuvImage::from_frame(&frame)
}

/// Decode a `grid` derived item: parse its payload, decode every referenced
/// tile (in parallel), and blit them row-major into one canvas.
fn decode_grid(
    container: &Container<'_>,
    item_id: u32,
) -> Result<(YuvImage, usize, usize), HeifError> {
    // ImageGrid payload (ISO 23008-12 §6.6.2.3.2).
    let payload = container.item_payload(item_id)?;
    let mut r = bytes::Reader::new(&payload, "grid item payload");
    let _version = r.u8()?;
    let flags = r.u8()?;
    let rows = r.u8()? as usize + 1;
    let cols = r.u8()? as usize + 1;
    // flags bit 0: output dimensions are 32-bit rather than 16-bit.
    let (out_w, out_h) = if flags & 1 != 0 {
        (r.u32()? as usize, r.u32()? as usize)
    } else {
        (r.u16()? as usize, r.u16()? as usize)
    };
    if out_w == 0 || out_h == 0 {
        return Err(HeifError::Invalid("grid output size is zero".into()));
    }

    let tiles = container.referenced_items(item_id, b"dimg");
    if tiles.len() != rows * cols {
        return Err(HeifError::Invalid(format!(
            "grid declares {rows}x{cols} tiles but references {}",
            tiles.len()
        )));
    }

    // Decode all tiles, splitting the list across the available cores —
    // each tile is an independent HEVC codestream. Results land in a
    // pre-sized Vec via disjoint per-thread chunks (no locking).
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let chunk_size = tiles.len().div_ceil(threads).max(1);
    let mut results: Vec<Option<Result<YuvImage, HeifError>>> = Vec::new();
    results.resize_with(tiles.len(), || None);
    crate::par::scope(|s| {
        for (tile_chunk, result_chunk) in
            tiles.chunks(chunk_size).zip(results.chunks_mut(chunk_size))
        {
            s.spawn(move || {
                for (tile_id, slot) in tile_chunk.iter().zip(result_chunk.iter_mut()) {
                    *slot = Some(decode_coded_item(container, *tile_id));
                }
            });
        }
    });

    let mut decoded = Vec::with_capacity(tiles.len());
    for slot in results {
        decoded.push(slot.expect("every tile chunk was processed")?);
    }

    // All tiles must agree on geometry; the canvas is a whole number of
    // tiles, cropped to (out_w, out_h) later in RGB space.
    let tile_w = decoded[0].width;
    let tile_h = decoded[0].height;
    if decoded
        .iter()
        .any(|t| t.width != tile_w || t.height != tile_h)
    {
        return Err(HeifError::Invalid("grid tiles have differing sizes".into()));
    }
    if tile_w * cols < out_w || tile_h * rows < out_h {
        return Err(HeifError::Invalid(format!(
            "grid canvas {}x{} smaller than declared output {out_w}x{out_h}",
            tile_w * cols,
            tile_h * rows
        )));
    }
    let mut canvas = YuvImage::black(tile_w * cols, tile_h * rows, decoded[0].bit_depth);
    for (i, tile) in decoded.iter().enumerate() {
        canvas.blit(tile, (i % cols) * tile_w, (i / cols) * tile_h)?;
    }
    Ok((canvas, out_w, out_h))
}

/// Colour description for the decode: the primary item's `colr` if present,
/// else the first grid tile's.
fn collect_color_info(container: &Container<'_>, primary: u32) -> ColorInfo {
    let mut info = ColorInfo::default();
    let visit = |item_id: u32, info: &mut ColorInfo| {
        for (_, property) in container.item_properties(item_id) {
            if let Property::Colr(c) = property {
                match c {
                    ColorProperty::Nclx {
                        primaries,
                        transfer,
                        matrix,
                        full_range,
                    } => {
                        if info.nclx.is_none() {
                            info.nclx = Some(Nclx {
                                primaries: *primaries,
                                transfer: *transfer,
                                matrix: *matrix,
                                full_range: *full_range,
                            });
                        }
                    }
                    ColorProperty::Icc => info.icc_present = true,
                }
            }
        }
    };
    visit(primary, &mut info);
    if info.nclx.is_none() {
        if let Some(first_tile) = container.referenced_items(primary, b"dimg").first() {
            visit(*first_tile, &mut info);
        }
    }
    info
}

/// Try to decode an alpha auxiliary image into a u16 plane (full-scale
/// 0..65535) matching the master's pre-transform output size. Any failure —
/// including the common monochrome-HEVC alpha rust_h265 can't decode —
/// returns `None` and the image ships without alpha.
fn decode_alpha_plane(
    container: &Container<'_>,
    primary: u32,
    out_w: usize,
    out_h: usize,
) -> Option<Vec<u16>> {
    // An alpha auxiliary points AT the image it augments: find an item
    // whose `auxl` reference targets the primary and whose auxC URN says
    // it's alpha.
    let aux_item = container.references.iter().find(|r| {
        &r.ref_type == b"auxl"
            && r.to_items.contains(&primary)
            && container
                .find_property(r.from_item, |p| match p {
                    Property::AuxC(urn) => {
                        Some(urn.contains("alpha") || urn == "urn:mpeg:hevc:2015:auxid:1")
                    }
                    _ => None,
                })
                .unwrap_or(false)
    })?;
    let (yuv, aux_w, aux_h) = decode_item_yuv(container, aux_item.from_item).ok()?;
    if aux_w != out_w || aux_h != out_h {
        return None;
    }
    // Alpha lives in the luma plane, full range (0 = transparent). Crop the
    // (possibly grid-)canvas to the output size and scale to u16.
    let max = ((1u32 << yuv.bit_depth) - 1) as f32;
    let plane = transform::crop_top_left(&yuv.y, yuv.width, 1, out_w, out_h);
    Some(
        plane
            .iter()
            .map(|&s| ((s as f32 / max) * 65535.0 + 0.5) as u16)
            .collect(),
    )
}

/// Interleave a u16 alpha plane into an 8-bit RGB buffer.
fn interleave_alpha8(rgb: &[u8], alpha: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
    for (px, &a) in rgb.chunks_exact(3).zip(alpha.iter()) {
        out.extend_from_slice(px);
        out.push((a >> 8) as u8);
    }
    out
}

/// Interleave a u16 alpha plane into a 16-bit RGB buffer.
fn interleave_alpha16(rgb: &[u16], alpha: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
    for (px, &a) in rgb.chunks_exact(3).zip(alpha.iter()) {
        out.extend_from_slice(px);
        out.push(a);
    }
    out
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

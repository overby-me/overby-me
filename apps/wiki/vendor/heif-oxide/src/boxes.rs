//! ISOBMFF / HEIF container parsing.
//!
//! Parses the box structure of a HEIF file (ISO/IEC 14496-12 base format,
//! ISO/IEC 23008-12 image format) into a [`Container`]: the primary item id,
//! per-item metadata (`iinf`), payload locations (`iloc`), inter-item
//! references (`iref`), and item properties (`iprp` = `ipco` + `ipma`).
//!
//! Only the still-image `meta` path is parsed — `moov`/track boxes (image
//! sequences) are ignored. All reads are bounds-checked; malformed input
//! yields errors, never panics.

use std::collections::HashMap;

use crate::bytes::{fourcc_str, Reader};
use crate::error::HeifError;

/// Brands (major or compatible) we accept in `ftyp`. Everything here stores
/// still images with HEVC-decodable payloads or is the codec-agnostic
/// structural brand (`mif1`); the actual codec check happens per-item.
const ACCEPTED_BRANDS: [&[u8; 4]; 8] = [
    b"heic", b"heix", b"heim", b"heis", // HEVC still image brands
    b"hevc", b"hevx", // HEVC image sequence brands (often listed as compatible)
    b"mif1", b"msf1", // structural HEIF brands
];

/// One entry from `iinf`/`infe`: what kind of item this is.
#[derive(Debug, Clone)]
pub struct ItemInfo {
    /// Item type fourcc: `hvc1` (HEVC image), `grid`, `Exif`, `mime`, ...
    pub item_type: [u8; 4],
    /// Non-zero means the item is protected (encrypted) — unsupported.
    pub protection_index: u16,
}

/// One entry from `iloc`: where an item's payload bytes live.
#[derive(Debug, Clone)]
pub struct ItemLocation {
    /// 0 = extents are absolute file offsets, 1 = offsets into `idat`,
    /// 2 = offsets into another item (unsupported).
    pub construction_method: u8,
    /// Non-zero references an external file (`dref`) — unsupported.
    pub data_reference_index: u16,
    pub base_offset: u64,
    /// `(offset, length)` pairs, concatenated in order to form the payload.
    pub extents: Vec<(u64, u64)>,
}

/// The `colr` property payload.
#[derive(Debug, Clone)]
pub enum ColorProperty {
    /// On-screen colour description: CICP code points per ISO 23091-2
    /// (same numbering as H.273): colour primaries, transfer
    /// characteristics, matrix coefficients, and the video-full-range flag.
    Nclx {
        primaries: u16,
        transfer: u16,
        matrix: u16,
        full_range: bool,
    },
    /// An embedded ICC profile (`rICC`/`prof`). We note its presence but do
    /// not apply it.
    Icc,
}

/// A parsed entry from the `ipco` property container. Order matters:
/// `ipma` associations index into this list 1-based, and transformative
/// properties (`irot`/`imir`/`clap`) must be applied in association order.
#[derive(Debug, Clone)]
pub enum Property {
    /// HEVCDecoderConfigurationRecord — raw bytes, parsed later by `hvcc`.
    HvcC(Vec<u8>),
    /// Nominal (pre-transform) width/height of the item. Informational —
    /// decode uses the codestream's (conformance-cropped) dimensions and
    /// the grid payload's output size as the authoritative geometry.
    #[allow(dead_code)]
    Ispe {
        width: u32,
        height: u32,
    },
    /// Rotation by `angle * 90` degrees **anti-clockwise**.
    Irot {
        angle: u8,
    },
    /// Mirror: axis 0 = about a vertical axis (left↔right),
    /// axis 1 = about a horizontal axis (top↔bottom).
    Imir {
        axis: u8,
    },
    /// Clean aperture crop, stored as the raw numerator/denominator fields.
    Clap {
        width_n: u32,
        width_d: u32,
        height_n: u32,
        height_d: u32,
        horiz_off_n: i32,
        horiz_off_d: u32,
        vert_off_n: i32,
        vert_off_d: u32,
    },
    Colr(ColorProperty),
    /// Auxiliary image type URN (on alpha/depth aux items).
    AuxC(String),
    /// Bits-per-channel — parsed so it counts as "recognized" when marked
    /// essential, but otherwise unused.
    Pixi,
    /// Anything we don't understand; fatal only if marked essential.
    Other([u8; 4]),
}

/// One `iref` entry: `from_item` --(ref_type)--> `to_items`.
///
/// Directions that matter here: a `grid` item points **at** its tiles with
/// `dimg`; an alpha auxiliary item points **at** the image it augments with
/// `auxl`.
#[derive(Debug, Clone)]
pub struct ItemReference {
    pub ref_type: [u8; 4],
    pub from_item: u32,
    pub to_items: Vec<u32>,
}

/// Everything the decoder needs from the container, referencing (not
/// copying) the file bytes for payload extents.
pub struct Container<'a> {
    data: &'a [u8],
    pub primary_item: u32,
    pub items: HashMap<u32, ItemInfo>,
    pub locations: HashMap<u32, ItemLocation>,
    /// Payload of the `idat` box inside `meta`, for construction_method 1.
    idat: Option<(usize, usize)>,
    /// `ipco` properties in declaration order (indexed 1-based by `ipma`).
    pub properties: Vec<Property>,
    /// item id → `(essential, 1-based property index)` in association order.
    pub associations: HashMap<u32, Vec<(bool, u16)>>,
    pub references: Vec<ItemReference>,
}

impl<'a> Container<'a> {
    /// Iterate an item's associated properties in association order.
    /// Out-of-range indices (corrupt `ipma`) are skipped.
    pub fn item_properties(&self, item_id: u32) -> impl Iterator<Item = (bool, &Property)> {
        self.associations
            .get(&item_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .iter()
            .filter_map(|&(essential, idx)| {
                // ipma property indices are 1-based; 0 means "no property".
                self.properties
                    .get(idx.checked_sub(1)? as usize)
                    .map(|p| (essential, p))
            })
    }

    /// Find the first property of a kind on an item (association order).
    pub fn find_property<T, F: Fn(&Property) -> Option<T>>(&self, item_id: u32, f: F) -> Option<T> {
        self.item_properties(item_id).find_map(|(_, p)| f(p))
    }

    /// The items a source item references with `ref_type` (e.g. the tile
    /// ids of a `grid` via `dimg`), preserving reference order.
    pub fn referenced_items(&self, from_item: u32, ref_type: &[u8; 4]) -> Vec<u32> {
        self.references
            .iter()
            .filter(|r| r.from_item == from_item && &r.ref_type == ref_type)
            .flat_map(|r| r.to_items.iter().copied())
            .collect()
    }

    /// Assemble an item's payload bytes by concatenating its `iloc` extents.
    pub fn item_payload(&self, item_id: u32) -> Result<Vec<u8>, HeifError> {
        let info = self
            .items
            .get(&item_id)
            .ok_or_else(|| HeifError::Invalid(format!("item {item_id} not in iinf")))?;
        if info.protection_index != 0 {
            return Err(HeifError::Unsupported("protected (encrypted) items".into()));
        }
        let loc = self
            .locations
            .get(&item_id)
            .ok_or_else(|| HeifError::Invalid(format!("item {item_id} has no iloc entry")))?;
        if loc.data_reference_index != 0 {
            return Err(HeifError::Unsupported(
                "item data in an external file (data_reference_index != 0)".into(),
            ));
        }
        // Resolve the byte range extents index into: the whole file
        // (construction_method 0) or the idat payload (method 1).
        let source: &[u8] = match loc.construction_method {
            0 => self.data,
            1 => {
                let (start, len) = self.idat.ok_or_else(|| {
                    HeifError::Invalid("iloc references idat but meta has no idat box".into())
                })?;
                &self.data[start..start + len]
            }
            m => {
                return Err(HeifError::Unsupported(format!(
                    "iloc construction_method {m}"
                )))
            }
        };
        let mut payload = Vec::new();
        for &(offset, length) in &loc.extents {
            let start = loc
                .base_offset
                .checked_add(offset)
                .ok_or(HeifError::Truncated("iloc extent"))? as usize;
            // extent_length 0 means "to the end of the source".
            let end = if length == 0 {
                source.len()
            } else {
                start
                    .checked_add(length as usize)
                    .ok_or(HeifError::Truncated("iloc extent"))?
            };
            if end > source.len() || start > end {
                return Err(HeifError::Truncated("iloc extent"));
            }
            payload.extend_from_slice(&source[start..end]);
        }
        Ok(payload)
    }
}

/// Parse the top level of a HEIF file: verify `ftyp`, then parse `meta`.
pub fn parse(data: &[u8]) -> Result<Container<'_>, HeifError> {
    let mut container = Container {
        data,
        primary_item: 0,
        items: HashMap::new(),
        locations: HashMap::new(),
        idat: None,
        properties: Vec::new(),
        associations: HashMap::new(),
        references: Vec::new(),
    };
    let mut saw_ftyp = false;
    let mut saw_meta = false;
    let mut saw_moov = false;
    let mut saw_pitm = false;

    each_box(data, "top-level", &mut |fourcc, body| {
        match &fourcc {
            b"ftyp" => {
                check_brands(body)?;
                saw_ftyp = true;
            }
            // Ignore any boxes before ftyp (there shouldn't be any) and
            // don't parse meta until the brand check passed.
            b"meta" if saw_ftyp => {
                parse_meta(body, &mut container, &mut saw_pitm)?;
                saw_meta = true;
            }
            // Tracked only to give sequence-shaped files a better error.
            b"moov" => saw_moov = true,
            // mdat/free/... — payload access goes through iloc offsets.
            _ => {}
        }
        Ok(())
    })?;

    if !saw_ftyp {
        return Err(HeifError::NotHeif("no ftyp box found".into()));
    }
    if !saw_meta {
        // Sequence files (msf1 brand) store tracks in moov instead of a
        // still-image meta box.
        if saw_moov {
            return Err(HeifError::Unsupported(
                "HEIF image sequences (track-based files)".into(),
            ));
        }
        return Err(HeifError::MissingBox("meta"));
    }
    if !saw_pitm {
        return Err(HeifError::MissingBox("pitm"));
    }
    Ok(container)
}

/// Verify the `ftyp` major or compatible brands contain one we accept.
fn check_brands(body: &[u8]) -> Result<(), HeifError> {
    let mut r = Reader::new(body, "ftyp");
    let major = r.fourcc()?;
    r.skip(4)?; // minor_version
    let mut brands = vec![major];
    while r.remaining() >= 4 {
        brands.push(r.fourcc()?);
    }
    if brands.iter().any(|b| ACCEPTED_BRANDS.contains(&b)) {
        Ok(())
    } else {
        Err(HeifError::NotHeif(format!(
            "unrecognized brand '{}' (not an HEVC-coded HEIF)",
            fourcc_str(major)
        )))
    }
}

/// Walk a run of sibling boxes in `data`, invoking `f(fourcc, body)` for
/// each. Bodies borrow from `data`, so consumers needing absolute file
/// positions (idat) can recover them by pointer arithmetic.
#[allow(clippy::type_complexity)]
fn each_box(
    data: &[u8],
    context: &'static str,
    f: &mut dyn FnMut([u8; 4], &[u8]) -> Result<(), HeifError>,
) -> Result<(), HeifError> {
    let mut pos = 0usize;
    while pos < data.len() {
        // A box header is at least size(4) + fourcc(4).
        if data.len() - pos < 8 {
            return Err(HeifError::Truncated(context));
        }
        let mut r = Reader::new(&data[pos..], context);
        let size32 = r.u32()?;
        let fourcc = r.fourcc()?;
        let (body_start, box_size) = match size32 {
            // size 0: box extends to the end of the enclosing container.
            0 => (pos + 8, data.len() - pos),
            // size 1: 64-bit largesize follows the fourcc.
            1 => {
                let largesize = r.u64()?;
                if largesize < 16 {
                    return Err(HeifError::Invalid(format!(
                        "box '{}' has invalid largesize {largesize}",
                        fourcc_str(fourcc)
                    )));
                }
                (pos + 16, largesize as usize)
            }
            s if (s as usize) < 8 => {
                return Err(HeifError::Invalid(format!(
                    "box '{}' has invalid size {s}",
                    fourcc_str(fourcc)
                )));
            }
            s => (pos + 8, s as usize),
        };
        let box_end = pos
            .checked_add(box_size)
            .filter(|&e| e <= data.len())
            .ok_or(HeifError::Truncated(context))?;
        f(fourcc, &data[body_start..box_end])?;
        pos = box_end;
    }
    Ok(())
}

/// Parse the `meta` full box and its children.
fn parse_meta(
    body: &[u8],
    container: &mut Container<'_>,
    saw_pitm: &mut bool,
) -> Result<(), HeifError> {
    let mut r = Reader::new(body, "meta");
    r.full_box_header()?;
    let children = &body[r.pos()..];
    each_box(children, "meta", &mut |fourcc, child| {
        match &fourcc {
            b"pitm" => {
                let mut r = Reader::new(child, "pitm");
                let (version, _) = r.full_box_header()?;
                container.primary_item = if version == 0 {
                    r.u16()? as u32
                } else {
                    r.u32()?
                };
                *saw_pitm = true;
            }
            b"iinf" => parse_iinf(child, container)?,
            b"iloc" => parse_iloc(child, container)?,
            b"iprp" => parse_iprp(child, container)?,
            b"iref" => parse_iref(child, container)?,
            b"idat" => {
                // Record the idat payload's position within the *file* so
                // item_payload can slice it. `child` borrows from the same
                // allocation as container.data, so offsets can be derived
                // from pointer arithmetic safely.
                let start = child.as_ptr() as usize - container.data.as_ptr() as usize;
                container.idat = Some((start, child.len()));
            }
            // hdlr, dinf, pict handler checks — intentionally lenient.
            _ => {}
        }
        Ok(())
    })
}

/// `iinf` → `infe`*: item id → item type.
fn parse_iinf(body: &[u8], container: &mut Container<'_>) -> Result<(), HeifError> {
    let mut r = Reader::new(body, "iinf");
    let (version, _) = r.full_box_header()?;
    let _entry_count = if version == 0 {
        r.u16()? as u32
    } else {
        r.u32()?
    };
    let children = &body[r.pos()..];
    each_box(children, "iinf", &mut |fourcc, child| {
        if &fourcc != b"infe" {
            return Ok(());
        }
        let mut r = Reader::new(child, "infe");
        let (version, _) = r.full_box_header()?;
        // Versions 0/1 predate HEIF item types and never appear in image
        // files; skip them rather than misparse.
        if version < 2 {
            return Ok(());
        }
        let item_id = if version == 2 {
            r.u16()? as u32
        } else {
            r.u32()?
        };
        let protection_index = r.u16()?;
        let item_type = r.fourcc()?;
        container.items.insert(
            item_id,
            ItemInfo {
                item_type,
                protection_index,
            },
        );
        Ok(())
    })
}

/// `iloc`: item id → payload byte extents.
fn parse_iloc(body: &[u8], container: &mut Container<'_>) -> Result<(), HeifError> {
    let mut r = Reader::new(body, "iloc");
    let (version, _) = r.full_box_header()?;
    if version > 2 {
        return Err(HeifError::Unsupported(format!("iloc version {version}")));
    }
    let b = r.u8()?;
    let offset_size = b >> 4;
    let length_size = b & 0xF;
    let b = r.u8()?;
    let base_offset_size = b >> 4;
    // index_size exists in versions 1 and 2; reserved (must-ignore) in 0.
    let index_size = if version >= 1 { b & 0xF } else { 0 };
    let item_count = if version < 2 {
        r.u16()? as u32
    } else {
        r.u32()?
    };
    for _ in 0..item_count {
        let item_id = if version < 2 {
            r.u16()? as u32
        } else {
            r.u32()?
        };
        let construction_method = if version >= 1 {
            (r.u16()? & 0xF) as u8
        } else {
            0
        };
        let data_reference_index = r.u16()?;
        let base_offset = r.uint_sized(base_offset_size)?;
        let extent_count = r.u16()?;
        let mut extents = Vec::with_capacity(extent_count as usize);
        for _ in 0..extent_count {
            if index_size > 0 {
                // extent_index — used only by construction_method 2, which
                // we reject at payload time; skip it here.
                r.uint_sized(index_size)?;
            }
            let extent_offset = r.uint_sized(offset_size)?;
            let extent_length = r.uint_sized(length_size)?;
            extents.push((extent_offset, extent_length));
        }
        container.locations.insert(
            item_id,
            ItemLocation {
                construction_method,
                data_reference_index,
                base_offset,
                extents,
            },
        );
    }
    Ok(())
}

/// `iprp` = `ipco` (property list) + `ipma` (item → property associations).
fn parse_iprp(body: &[u8], container: &mut Container<'_>) -> Result<(), HeifError> {
    each_box(body, "iprp", &mut |fourcc, child| {
        match &fourcc {
            b"ipco" => {
                each_box(child, "ipco", &mut |prop_fourcc, prop_body| {
                    container
                        .properties
                        .push(parse_property(prop_fourcc, prop_body)?);
                    Ok(())
                })?;
            }
            b"ipma" => {
                let mut r = Reader::new(child, "ipma");
                let (version, flags) = r.full_box_header()?;
                let entry_count = r.u32()?;
                for _ in 0..entry_count {
                    let item_id = if version < 1 {
                        r.u16()? as u32
                    } else {
                        r.u32()?
                    };
                    let association_count = r.u8()?;
                    let mut assocs = Vec::with_capacity(association_count as usize);
                    for _ in 0..association_count {
                        // flags bit 0 selects 15-bit (2-byte) vs 7-bit
                        // (1-byte) property indices; MSB = essential.
                        let (essential, index) = if flags & 1 != 0 {
                            let v = r.u16()?;
                            (v & 0x8000 != 0, v & 0x7FFF)
                        } else {
                            let v = r.u8()? as u16;
                            (v & 0x80 != 0, v & 0x7F)
                        };
                        assocs.push((essential, index));
                    }
                    container.associations.insert(item_id, assocs);
                }
            }
            _ => {}
        }
        Ok(())
    })
}

/// Parse one property box from `ipco`.
fn parse_property(fourcc: [u8; 4], body: &[u8]) -> Result<Property, HeifError> {
    Ok(match &fourcc {
        b"hvcC" => Property::HvcC(body.to_vec()),
        b"ispe" => {
            let mut r = Reader::new(body, "ispe");
            r.full_box_header()?;
            Property::Ispe {
                width: r.u32()?,
                height: r.u32()?,
            }
        }
        b"irot" => {
            let mut r = Reader::new(body, "irot");
            Property::Irot {
                angle: r.u8()? & 0x3,
            }
        }
        b"imir" => {
            let mut r = Reader::new(body, "imir");
            Property::Imir {
                axis: r.u8()? & 0x1,
            }
        }
        b"clap" => {
            let mut r = Reader::new(body, "clap");
            Property::Clap {
                width_n: r.u32()?,
                width_d: r.u32()?,
                height_n: r.u32()?,
                height_d: r.u32()?,
                horiz_off_n: r.u32()? as i32,
                horiz_off_d: r.u32()?,
                vert_off_n: r.u32()? as i32,
                vert_off_d: r.u32()?,
            }
        }
        b"colr" => {
            let mut r = Reader::new(body, "colr");
            let colour_type = r.fourcc()?;
            match &colour_type {
                b"nclx" => Property::Colr(ColorProperty::Nclx {
                    primaries: r.u16()?,
                    transfer: r.u16()?,
                    matrix: r.u16()?,
                    full_range: r.u8()? & 0x80 != 0,
                }),
                b"rICC" | b"prof" => Property::Colr(ColorProperty::Icc),
                _ => Property::Other(fourcc),
            }
        }
        b"auxC" => {
            let mut r = Reader::new(body, "auxC");
            r.full_box_header()?;
            Property::AuxC(r.c_string()?.to_string())
        }
        b"pixi" => Property::Pixi,
        _ => Property::Other(fourcc),
    })
}

/// `iref`: typed item-to-item references.
fn parse_iref(body: &[u8], container: &mut Container<'_>) -> Result<(), HeifError> {
    let mut r = Reader::new(body, "iref");
    let (version, _) = r.full_box_header()?;
    let children = &body[r.pos()..];
    each_box(children, "iref", &mut |ref_type, child| {
        let mut r = Reader::new(child, "iref entry");
        let from_item = if version == 0 {
            r.u16()? as u32
        } else {
            r.u32()?
        };
        let count = r.u16()?;
        let mut to_items = Vec::with_capacity(count as usize);
        for _ in 0..count {
            to_items.push(if version == 0 {
                r.u16()? as u32
            } else {
                r.u32()?
            });
        }
        container.references.push(ItemReference {
            ref_type,
            from_item,
            to_items,
        });
        Ok(())
    })
}

#[cfg(test)]
#[path = "boxes_tests.rs"]
mod tests;

//! Test-only builder that assembles minimal-but-valid HEIC files in memory.
//!
//! Used by the unit/integration tests to exercise the container parser and
//! the full decode path without committing opaque binary fixtures for every
//! case: tests combine this builder with tiny committed `.h265` codestreams.
//!
//! Deliberately independent of the parser's internals — it writes boxes
//! straight from the ISO 14496-12 / 23008-12 wire layout, so a shared
//! misconception between builder and parser is at least a *second* reading
//! of the spec, and the end-to-end tests double-check against real-file
//! behaviour (a committed iPhone-style fixture).

/// One item to place in the file.
pub struct TestItem {
    pub id: u32,
    pub item_type: [u8; 4],
    /// Bytes stored in `mdat` and referenced through `iloc`.
    pub payload: Vec<u8>,
    /// Complete property boxes (already serialized), with their essential
    /// flag for `ipma`.
    pub props: Vec<(bool, Vec<u8>)>,
}

/// `(reference_type, from_item, to_items)` entries for `iref`.
pub type TestRef = ([u8; 4], u32, Vec<u32>);

fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

/// Serialize a plain box: size + fourcc + body.
pub fn plain_box(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&be32(8 + body.len() as u32));
    out.extend_from_slice(fourcc);
    out.extend_from_slice(body);
    out
}

/// Serialize a full box: size + fourcc + version + flags + body.
pub fn full_box(fourcc: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut inner = Vec::with_capacity(4 + body.len());
    inner.push(version);
    inner.extend_from_slice(&be32(flags)[1..]);
    inner.extend_from_slice(body);
    plain_box(fourcc, &inner)
}

/// An `ispe` (spatial extents) property box.
pub fn ispe(width: u32, height: u32) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&be32(width));
    body.extend_from_slice(&be32(height));
    full_box(b"ispe", 0, 0, &body)
}

/// An `irot` property box (angle in 90° CCW steps).
pub fn irot(angle: u8) -> Vec<u8> {
    plain_box(b"irot", &[angle & 0x3])
}

/// An `imir` property box (axis 0 = vertical axis / left-right).
pub fn imir(axis: u8) -> Vec<u8> {
    plain_box(b"imir", &[axis & 0x1])
}

/// A `colr` nclx property box.
pub fn colr_nclx(primaries: u16, transfer: u16, matrix: u16, full_range: bool) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"nclx");
    body.extend_from_slice(&be16(primaries));
    body.extend_from_slice(&be16(transfer));
    body.extend_from_slice(&be16(matrix));
    body.push(if full_range { 0x80 } else { 0 });
    plain_box(b"colr", &body)
}

/// An `hvcC` property box wrapping parameter-set NAL units, one array per
/// NAL. `nal_length_size` is the byte width of the item payload's length
/// prefixes (4 in everything Apple writes).
pub fn hvcc(param_sets: &[(u8, &[u8])], nal_length_size: u8) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(1); // configurationVersion
    body.extend_from_slice(&[0u8; 20]); // profile/level fields — unused by the parser
    body.push(0xFC | (nal_length_size - 1)); // reserved bits + lengthSizeMinusOne
    body.push(param_sets.len() as u8); // numOfArrays
    for (nal_type, data) in param_sets {
        body.push(nal_type & 0x3F); // array_completeness=0 + NAL_unit_type
        body.extend_from_slice(&be16(1)); // numNalus
        body.extend_from_slice(&be16(data.len() as u16));
        body.extend_from_slice(data);
    }
    plain_box(b"hvcC", &body)
}

/// A `grid` item payload (not a box — item payload bytes).
pub fn grid_payload(rows: u8, cols: u8, out_w: u16, out_h: u16) -> Vec<u8> {
    let mut out = vec![0, 0, rows - 1, cols - 1];
    out.extend_from_slice(&be16(out_w));
    out.extend_from_slice(&be16(out_h));
    out
}

/// Convert an Annex B codestream into `(hvcC property, length-prefixed item
/// payload)` the way a HEIF muxer would: parameter-set NALs (VPS/SPS/PPS)
/// go into `hvcC`, everything else becomes 4-byte-length-prefixed payload.
pub fn annex_b_to_item(annex_b: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut param_sets: Vec<(u8, Vec<u8>)> = Vec::new();
    let mut payload = Vec::new();
    for nal in split_annex_b(annex_b) {
        let nal_type = (nal[0] >> 1) & 0x3F;
        match nal_type {
            32..=34 => param_sets.push((nal_type, nal.to_vec())), // VPS/SPS/PPS
            _ => {
                payload.extend_from_slice(&be32(nal.len() as u32));
                payload.extend_from_slice(nal);
            }
        }
    }
    let sets: Vec<(u8, &[u8])> = param_sets.iter().map(|(t, d)| (*t, d.as_slice())).collect();
    (hvcc(&sets, 4), payload)
}

/// Minimal Annex B splitter (handles 3- and 4-byte start codes) so the
/// builder doesn't depend on the code under test.
fn split_annex_b(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut nals = Vec::new();
    for (idx, &start) in starts.iter().enumerate() {
        let mut end = if idx + 1 < starts.len() {
            starts[idx + 1] - 3
        } else {
            data.len()
        };
        // A 4-byte start code leaves a trailing zero before the next one.
        while end > start && data[end - 1] == 0 {
            end -= 1;
        }
        nals.push(&data[start..end]);
    }
    nals
}

/// Assemble a complete HEIC file: `ftyp` + `meta` (pitm/iinf/iloc/iprp/iref)
/// + `mdat`, with `iloc` using absolute file offsets (construction method 0).
pub fn make_heic(items: &[TestItem], primary: u32, refs: &[TestRef]) -> Vec<u8> {
    make_heic_with_brand(items, primary, refs, b"heic")
}

/// Same as [`make_heic`] with a chosen major brand (for brand-check tests).
pub fn make_heic_with_brand(
    items: &[TestItem],
    primary: u32,
    refs: &[TestRef],
    brand: &[u8; 4],
) -> Vec<u8> {
    let mut ftyp_body = Vec::new();
    ftyp_body.extend_from_slice(brand);
    ftyp_body.extend_from_slice(&be32(0)); // minor_version
    ftyp_body.extend_from_slice(brand); // compatible brand (major only —
                                        // brand-rejection tests rely on it)
    let ftyp = plain_box(b"ftyp", &ftyp_body);

    // The meta box's size doesn't depend on the offset *values* (fixed-width
    // fields), so build once with zero offsets to learn the mdat position,
    // then rebuild with real offsets.
    let meta_size = build_meta(items, primary, refs, 0).len();
    let mdat_payload_start = ftyp.len() + meta_size + 8; // + mdat header
    let meta = build_meta(items, primary, refs, mdat_payload_start as u32);

    let mdat_body: Vec<u8> = items.iter().flat_map(|i| i.payload.clone()).collect();
    let mdat = plain_box(b"mdat", &mdat_body);

    let mut file = ftyp;
    file.extend_from_slice(&meta);
    file.extend_from_slice(&mdat);
    file
}

fn build_meta(items: &[TestItem], primary: u32, refs: &[TestRef], mdat_start: u32) -> Vec<u8> {
    // pitm (version 0, 16-bit id)
    let pitm = full_box(b"pitm", 0, 0, &be16(primary as u16));

    // iinf with one infe (version 2) per item
    let mut iinf_body = Vec::new();
    iinf_body.extend_from_slice(&be16(items.len() as u16));
    for item in items {
        let mut infe_body = Vec::new();
        infe_body.extend_from_slice(&be16(item.id as u16));
        infe_body.extend_from_slice(&be16(0)); // item_protection_index
        infe_body.extend_from_slice(&item.item_type);
        infe_body.push(0); // empty item_name
        iinf_body.extend_from_slice(&full_box(b"infe", 2, 0, &infe_body));
    }
    let iinf = full_box(b"iinf", 0, 0, &iinf_body);

    // iloc (version 0): offset_size 4, length_size 4, base_offset_size 0.
    let mut iloc_body = Vec::new();
    iloc_body.push(0x44);
    iloc_body.push(0x00);
    iloc_body.extend_from_slice(&be16(items.len() as u16));
    let mut running_offset = mdat_start;
    for item in items {
        iloc_body.extend_from_slice(&be16(item.id as u16));
        iloc_body.extend_from_slice(&be16(0)); // data_reference_index
        iloc_body.extend_from_slice(&be16(1)); // extent_count
        iloc_body.extend_from_slice(&be32(running_offset));
        iloc_body.extend_from_slice(&be32(item.payload.len() as u32));
        running_offset += item.payload.len() as u32;
    }
    let iloc = full_box(b"iloc", 0, 0, &iloc_body);

    // iprp: ipco lists every item's properties in order; ipma associates
    // them back by global 1-based index.
    let mut ipco_body = Vec::new();
    let mut ipma_body = Vec::new();
    ipma_body.extend_from_slice(&be32(items.len() as u32));
    let mut prop_index = 0u8;
    for item in items {
        ipma_body.extend_from_slice(&be16(item.id as u16));
        ipma_body.push(item.props.len() as u8);
        for (essential, prop_box) in &item.props {
            ipco_body.extend_from_slice(prop_box);
            prop_index += 1;
            ipma_body.push(if *essential {
                0x80 | prop_index
            } else {
                prop_index
            });
        }
    }
    let ipco = plain_box(b"ipco", &ipco_body);
    let ipma = full_box(b"ipma", 0, 0, &ipma_body);
    let mut iprp_body = ipco;
    iprp_body.extend_from_slice(&ipma);
    let iprp = plain_box(b"iprp", &iprp_body);

    // iref (version 0, 16-bit ids)
    let mut iref_children = Vec::new();
    for (ref_type, from, to) in refs {
        let mut body = Vec::new();
        body.extend_from_slice(&be16(*from as u16));
        body.extend_from_slice(&be16(to.len() as u16));
        for id in to {
            body.extend_from_slice(&be16(*id as u16));
        }
        iref_children.extend_from_slice(&plain_box(ref_type, &body));
    }
    let iref = full_box(b"iref", 0, 0, &iref_children);

    let mut meta_body = pitm;
    meta_body.extend_from_slice(&iinf);
    meta_body.extend_from_slice(&iloc);
    meta_body.extend_from_slice(&iprp);
    if !refs.is_empty() {
        meta_body.extend_from_slice(&iref);
    }
    full_box(b"meta", 0, 0, &meta_body)
}

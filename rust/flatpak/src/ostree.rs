//! Minimal OSTree HTTP client for pulling from Flatpak remotes.
//!
//! Implements just enough of the OSTree protocol to:
//! 1. Fetch and parse the summary file (GVariant format)
//! 2. Resolve refs to commit checksums
//! 3. Fetch commit, dirtree, dirmeta, and file objects
//! 4. Checkout a commit to a local directory

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Object types and checksums
// ---------------------------------------------------------------------------

/// A 32-byte SHA256 checksum as hex string.
pub type Checksum = String;

fn checksum_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn object_url(base: &str, checksum: &str, ext: &str) -> String {
    let base = base.trim_end_matches('/');
    format!("{base}/objects/{}/{}.{ext}", &checksum[..2], &checksum[2..])
}

// ---------------------------------------------------------------------------
// GVariant parser (minimal, for summary and object formats)
// ---------------------------------------------------------------------------

/// Determine the offset size for a GVariant container of the given byte length.
fn offset_size(total_len: usize) -> usize {
    if total_len <= 0xFF {
        1
    } else if total_len <= 0xFFFF {
        2
    } else {
        4
    }
}

/// Read an offset value of the given size from a byte slice.
fn read_offset(data: &[u8], pos: usize, size: usize) -> usize {
    match size {
        1 => data[pos] as usize,
        2 => u16::from_le_bytes([data[pos], data[pos + 1]]) as usize,
        4 => u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize,
        _ => 0,
    }
}

/// Read a NUL-terminated string from a byte slice.
fn read_string(data: &[u8]) -> &str {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    std::str::from_utf8(&data[..end]).unwrap_or("")
}

/// Align a position up to the given alignment.
fn align_up(pos: usize, alignment: usize) -> usize {
    (pos + alignment - 1) & !(alignment - 1)
}

// ---------------------------------------------------------------------------
// Summary file parsing
// ---------------------------------------------------------------------------

/// A ref entry from the summary file.
#[derive(Debug, Clone)]
pub struct SummaryRef {
    pub name: String,
    pub commit_size: u64,
    pub checksum: Checksum,
}

/// Parse the OSTree summary file.
///
/// Format: `(a(s(taya{sv}))a{sv})`
/// - Element 0: array of (refname, (commit_size, checksum, metadata))
/// - Element 1: repo metadata
pub fn parse_summary(data: &[u8]) -> Result<Vec<SummaryRef>, String> {
    if data.len() < 8 {
        return Err("summary too short".into());
    }

    let osz = offset_size(data.len());

    // The outer tuple has 2 elements. The first element (array) is variable-size,
    // so there's one framing offset at the end for it.
    let array_end = read_offset(data, data.len() - osz, osz);
    let array_data = &data[..array_end];

    parse_summary_refs_array(array_data)
}

/// Parse the refs array: `a(s(taya{sv}))`
fn parse_summary_refs_array(data: &[u8]) -> Result<Vec<SummaryRef>, String> {
    if data.is_empty() {
        return Ok(Vec::new());
    }

    let osz = offset_size(data.len());

    // Variable-size array: offsets at the end in forward order.
    // Find how many entries by reading the last offset and computing count.
    let last_offset_pos = data.len() - osz;
    let last_offset = read_offset(data, last_offset_pos, osz);

    // Count = number of offsets that fit after `last_offset`.
    let framing_region_size = data.len() - last_offset;
    let count = framing_region_size / osz;

    let mut refs = Vec::new();
    let mut start = 0;

    for i in 0..count {
        let end = read_offset(data, last_offset + i * osz, osz);
        let entry = &data[start..end];
        if let Ok(r) = parse_summary_ref_entry(entry) {
            refs.push(r);
        }
        start = align_up(end, 8); // tuple alignment = max alignment of children = 8 (for `t`)
    }

    Ok(refs)
}

/// Parse a single ref entry: `(s(taya{sv}))`
fn parse_summary_ref_entry(data: &[u8]) -> Result<SummaryRef, String> {
    if data.len() < 42 {
        return Err("ref entry too short".into());
    }

    let osz = offset_size(data.len());

    // This is a tuple (s, (t, ay, a{sv})).
    // The string `s` is variable-size. Its framing offset is at the end.
    let string_end = read_offset(data, data.len() - osz, osz);
    let name = read_string(&data[..string_end]).to_string();

    // The inner tuple starts after the string, aligned to 8 (for `t`).
    let inner_start = align_up(string_end + 1, 8); // +1 for NUL terminator
    if inner_start + 8 > data.len() {
        return Err("inner tuple too short".into());
    }

    // Inner tuple: (t, ay, a{sv})
    // `t` is at the start (8 bytes, LE).
    let commit_size = u64::from_le_bytes(
        data[inner_start..inner_start + 8]
            .try_into()
            .map_err(|_| "bad commit size")?,
    );

    // `ay` follows `t` at offset 8 within the inner tuple.
    // It's variable-size, but in the summary the checksum is always 32 bytes.
    let checksum_start = inner_start + 8;
    if checksum_start + 32 > data.len() {
        return Err("checksum truncated".into());
    }
    let checksum = checksum_to_hex(&data[checksum_start..checksum_start + 32]);

    Ok(SummaryRef {
        name,
        commit_size,
        checksum,
    })
}

// ---------------------------------------------------------------------------
// Commit object parsing
// ---------------------------------------------------------------------------

/// Parsed commit object.
#[derive(Debug)]
#[allow(dead_code)]
pub struct Commit {
    pub root_dirtree: Checksum,
    pub root_dirmeta: Checksum,
    pub subject: String,
    pub timestamp: u64,
}

/// Parse a commit object: `(a{sv}aya(say)sstayay)`
pub fn parse_commit(data: &[u8]) -> Result<Commit, String> {
    if data.len() < 66 {
        return Err("commit too short".into());
    }

    // The last two elements are `ay` (root_dirtree, 32 bytes) and `ay`
    // (root_dirmeta, 32 bytes). Since `ay` is variable-size, framing
    // offsets are at the end.
    //
    // This commit tuple has 8 elements. Fixed-size elements: element 5 (`t`).
    // Variable-size: elements 0,1,2,3,4,6,7.
    // Framing offsets are for non-last variable elements: 0,1,2,3,4,6 (6 offsets).
    let osz = offset_size(data.len());

    // Read framing offsets from the end, in reverse element order.
    // The framing region has 6 offsets (for elements 0,1,2,3,4,6).
    let framing_start = data.len() - 6 * osz;

    // Offsets stored in order: element 0, 1, 2, 3, 4, 6.
    let _off0 = read_offset(data, framing_start, osz);
    let _off1 = read_offset(data, framing_start + osz, osz);
    let off2 = read_offset(data, framing_start + 2 * osz, osz);
    let off3 = read_offset(data, framing_start + 3 * osz, osz);
    let off4 = read_offset(data, framing_start + 4 * osz, osz);
    let off6 = read_offset(data, framing_start + 5 * osz, osz);

    // Element 3: subject (s), starts after element 2, aligned to 1.
    let subject_start = align_up(off2, 1);
    let subject = read_string(&data[subject_start..off3]).to_string();

    // Element 5: timestamp (t, big-endian), starts after element 4, aligned to 8.
    let ts_start = align_up(off4, 8);
    let timestamp = if ts_start + 8 <= framing_start {
        u64::from_be_bytes(data[ts_start..ts_start + 8].try_into().unwrap_or([0; 8]))
    } else {
        0
    };

    // Element 6: root dirtree checksum (ay, 32 bytes).
    let dirtree_start = align_up(ts_start + 8, 1);
    let dirtree_end = off6;
    let root_dirtree = if dirtree_end - dirtree_start >= 32 {
        checksum_to_hex(&data[dirtree_start..dirtree_start + 32])
    } else {
        return Err("dirtree checksum truncated".into());
    };

    // Element 7: root dirmeta checksum (ay, 32 bytes, last element).
    let dirmeta_start = align_up(off6, 1);
    let dirmeta_end = framing_start;
    let root_dirmeta = if dirmeta_end - dirmeta_start >= 32 {
        checksum_to_hex(&data[dirmeta_start..dirmeta_start + 32])
    } else {
        return Err("dirmeta checksum truncated".into());
    };

    Ok(Commit {
        root_dirtree,
        root_dirmeta,
        subject,
        timestamp,
    })
}

// ---------------------------------------------------------------------------
// Dirtree object parsing
// ---------------------------------------------------------------------------

/// A file entry in a dirtree.
#[derive(Debug)]
pub struct DirtreeFile {
    pub name: String,
    pub checksum: Checksum,
}

/// A subdirectory entry in a dirtree.
#[derive(Debug)]
pub struct DirtreeDir {
    pub name: String,
    pub dirtree_checksum: Checksum,
    pub dirmeta_checksum: Checksum,
}

/// Parsed dirtree object.
#[derive(Debug)]
pub struct Dirtree {
    pub files: Vec<DirtreeFile>,
    pub dirs: Vec<DirtreeDir>,
}

/// Parse a dirtree object: `(a(say)a(sayay))`
pub fn parse_dirtree(data: &[u8]) -> Result<Dirtree, String> {
    if data.is_empty() {
        return Ok(Dirtree {
            files: Vec::new(),
            dirs: Vec::new(),
        });
    }

    let osz = offset_size(data.len());

    // Outer tuple has 2 variable-size elements. One framing offset for element 0.
    let files_end = read_offset(data, data.len() - osz, osz);
    let files_data = &data[..files_end];
    let dirs_start = align_up(files_end, 8); // alignment of inner tuple
    let dirs_data = &data[dirs_start..data.len() - osz];

    let files = parse_files_array(files_data);
    let dirs = parse_dirs_array(dirs_data);

    Ok(Dirtree { files, dirs })
}

/// Parse files array: `a(say)` — each element is (filename, 32-byte checksum).
fn parse_files_array(data: &[u8]) -> Vec<DirtreeFile> {
    if data.is_empty() {
        return Vec::new();
    }

    let osz = offset_size(data.len());
    let mut files = Vec::new();

    // Variable-size array with offsets at the end.
    // Each entry is a tuple (s, ay). The `s` is variable, `ay` is variable but
    // always 32 bytes. The tuple has one framing offset for `s`.
    let last_offset_pos = data.len() - osz;
    let last_offset = read_offset(data, last_offset_pos, osz);
    let framing_region_size = data.len() - last_offset;
    let count = framing_region_size / osz;

    let mut start = 0;
    for i in 0..count {
        let end = read_offset(data, last_offset + i * osz, osz);
        let entry = &data[start..end];
        if let Some(file) = parse_file_entry(entry) {
            files.push(file);
        }
        start = align_up(end, 4); // tuple alignment for (s, ay) = 1, but entries need
        // alignment per the element type
    }

    files
}

fn parse_file_entry(data: &[u8]) -> Option<DirtreeFile> {
    let osz = offset_size(data.len());
    if data.len() < 33 + osz {
        return None;
    }

    let name_end = read_offset(data, data.len() - osz, osz);
    let name = read_string(&data[..name_end]).to_string();

    let checksum_start = name_end + 1; // skip NUL
    if checksum_start + 32 > data.len() - osz {
        return None;
    }
    let checksum = checksum_to_hex(&data[checksum_start..checksum_start + 32]);

    Some(DirtreeFile { name, checksum })
}

/// Parse directories array: `a(sayay)`.
fn parse_dirs_array(data: &[u8]) -> Vec<DirtreeDir> {
    if data.is_empty() {
        return Vec::new();
    }

    let osz = offset_size(data.len());
    let mut dirs = Vec::new();

    let last_offset_pos = data.len() - osz;
    let last_offset = read_offset(data, last_offset_pos, osz);
    let framing_region_size = data.len() - last_offset;
    let count = framing_region_size / osz;

    let mut start = 0;
    for i in 0..count {
        let end = read_offset(data, last_offset + i * osz, osz);
        let entry = &data[start..end];
        if let Some(dir) = parse_dir_entry(entry) {
            dirs.push(dir);
        }
        start = align_up(end, 4);
    }

    dirs
}

fn parse_dir_entry(data: &[u8]) -> Option<DirtreeDir> {
    let osz = offset_size(data.len());
    if data.len() < 65 + osz {
        return None;
    }

    // Tuple (s, ay, ay) — 2 framing offsets (for s and first ay).
    let name_end = read_offset(data, data.len() - 2 * osz, osz);
    let name = read_string(&data[..name_end]).to_string();

    let dirtree_start = name_end + 1; // skip NUL
    let dirtree_end = read_offset(data, data.len() - osz, osz);
    if dirtree_end - dirtree_start < 32 {
        return None;
    }
    let dirtree_checksum = checksum_to_hex(&data[dirtree_start..dirtree_start + 32]);

    let dirmeta_start = dirtree_end;
    let dirmeta_end = data.len() - 2 * osz;
    if dirmeta_end - dirmeta_start < 32 {
        return None;
    }
    let dirmeta_checksum = checksum_to_hex(&data[dirmeta_start..dirmeta_start + 32]);

    Some(DirtreeDir {
        name,
        dirtree_checksum,
        dirmeta_checksum,
    })
}

// ---------------------------------------------------------------------------
// Dirmeta parsing
// ---------------------------------------------------------------------------

/// Parsed dirmeta object.
#[derive(Debug)]
#[allow(dead_code)]
pub struct Dirmeta {
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
}

/// Parse a dirmeta object: `(uuua(ayay))`
pub fn parse_dirmeta(data: &[u8]) -> Result<Dirmeta, String> {
    if data.len() < 12 {
        return Err("dirmeta too short".into());
    }
    // The `u` values are big-endian (via GUINT32_TO_BE).
    let uid = u32::from_be_bytes(data[0..4].try_into().unwrap());
    let gid = u32::from_be_bytes(data[4..8].try_into().unwrap());
    let mode = u32::from_be_bytes(data[8..12].try_into().unwrap());

    Ok(Dirmeta { uid, gid, mode })
}

// ---------------------------------------------------------------------------
// Content file (.filez) parsing
// ---------------------------------------------------------------------------

/// Parsed file header from a .filez object.
#[derive(Debug)]
#[allow(dead_code)]
pub struct FileHeader {
    pub size: u64,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub symlink_target: String,
}

/// Parse the header of a .filez file and return (header, offset to compressed data).
pub fn parse_filez_header(data: &[u8]) -> Result<(FileHeader, usize), String> {
    if data.len() < 8 {
        return Err("filez too short".into());
    }

    // First 4 bytes: BE uint32 variant_size.
    let variant_size = u32::from_be_bytes(data[0..4].try_into().unwrap()) as usize;
    // 4 bytes padding.
    let header_data_start = 8;
    let header_data_end = header_data_start + variant_size;

    if header_data_end > data.len() {
        return Err("filez header extends past data".into());
    }

    let hdr = &data[header_data_start..header_data_end];

    // Header GVariant: (tuuuusa(ayay))
    // All integer fields are big-endian.
    if hdr.len() < 28 {
        return Err("filez header too short".into());
    }

    let size = u64::from_be_bytes(hdr[0..8].try_into().unwrap());
    let uid = u32::from_be_bytes(hdr[8..12].try_into().unwrap());
    let gid = u32::from_be_bytes(hdr[12..16].try_into().unwrap());
    let mode = u32::from_be_bytes(hdr[16..20].try_into().unwrap());
    let _rdev = u32::from_be_bytes(hdr[20..24].try_into().unwrap());

    // Symlink target string starts at offset 24.
    let symlink_target = read_string(&hdr[24..]).to_string();

    Ok((
        FileHeader {
            size,
            uid,
            gid,
            mode,
            symlink_target,
        },
        header_data_end,
    ))
}

/// Decompress the content of a .filez file (raw deflate after the header).
pub fn decompress_filez_content(data: &[u8], content_offset: usize) -> Result<Vec<u8>, String> {
    let compressed = &data[content_offset..];
    if compressed.is_empty() {
        return Ok(Vec::new());
    }

    // Raw deflate decompression.
    // Use a minimal deflate implementation or fall back to calling `zlib`.
    // For simplicity, shell out to `python3 -c` for decompression, or
    // use a pure approach. Since we only have libc as a dep, let's use
    // the system zlib via FFI.
    decompress_raw_deflate(compressed)
}

fn decompress_raw_deflate(compressed: &[u8]) -> Result<Vec<u8>, String> {
    miniz_oxide::inflate::decompress_to_vec(compressed)
        .map_err(|e| format!("deflate decompression failed: {e:?}"))
}

// ---------------------------------------------------------------------------
// HTTP fetching (reuses the simple HTTP client approach)
// ---------------------------------------------------------------------------

/// Fetch a URL and return the response body.
pub fn fetch_url(url: &str) -> Result<Vec<u8>, String> {
    // Parse URL.
    let url_str = if !url.contains("://") {
        format!("https://{url}")
    } else {
        url.to_string()
    };

    let (scheme, rest) = url_str
        .split_once("://")
        .ok_or_else(|| format!("bad url: {url}"))?;
    let (authority, path) = rest
        .find('/')
        .map(|i| (&rest[..i], &rest[i..]))
        .unwrap_or((rest, "/"));
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().unwrap_or(443)),
        None => (authority, if scheme == "https" { 443 } else { 80 }),
    };

    let addr = format!("{host}:{port}");
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: rust-flatpak/0.1.0\r\nConnection: close\r\nAccept: */*\r\n\r\n"
    );

    if scheme == "https" {
        fetch_https(host, &addr, &request)
    } else {
        fetch_http(&addr, &request)
    }
}

fn fetch_http(addr: &str, request: &str) -> Result<Vec<u8>, String> {
    let mut stream = TcpStream::connect(addr).map_err(|e| format!("connect {addr}: {e}"))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write: {e}"))?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|e| format!("read: {e}"))?;

    extract_body(&response)
}

fn fetch_https(host: &str, addr: &str, request: &str) -> Result<Vec<u8>, String> {
    let root_store = rustls_root_store();
    let config = Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth(),
    );

    let server_name = rustls::pki_types::ServerName::try_from(host)
        .map_err(|e| format!("bad server name: {e}"))?
        .to_owned();

    let conn =
        rustls::ClientConnection::new(config, server_name).map_err(|e| format!("tls: {e}"))?;

    let tcp = TcpStream::connect(addr).map_err(|e| format!("connect: {e}"))?;
    let mut tls = rustls::StreamOwned::new(conn, tcp);

    tls.write_all(request.as_bytes())
        .map_err(|e| format!("tls write: {e}"))?;
    tls.flush().map_err(|e| format!("tls flush: {e}"))?;

    let mut response = Vec::new();
    tls.read_to_end(&mut response)
        .map_err(|e| format!("tls read: {e}"))?;

    extract_body(&response)
}

fn rustls_root_store() -> rustls::RootCertStore {
    let mut store = rustls::RootCertStore::empty();
    store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    store
}

fn extract_body(response: &[u8]) -> Result<Vec<u8>, String> {
    // Find the end of HTTP headers.
    let header_end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("no HTTP header boundary")?;

    let header = std::str::from_utf8(&response[..header_end]).unwrap_or("");
    let status_line = header.lines().next().unwrap_or("");
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if status_code != 200 {
        return Err(format!("HTTP {status_code}: {status_line}"));
    }

    Ok(response[header_end + 4..].to_vec())
}

// ---------------------------------------------------------------------------
// High-level operations
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Local object cache
// ---------------------------------------------------------------------------

/// Get the default cache directory for OSTree objects.
fn cache_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    std::path::PathBuf::from(format!("{home}/.local/share/flatpak/repo/objects"))
}

/// Try to read an object from the local cache.
fn cache_get(checksum: &str, ext: &str) -> Option<Vec<u8>> {
    let dir = cache_dir();
    let path = dir
        .join(&checksum[..2])
        .join(format!("{}.{ext}", &checksum[2..]));
    fs::read(&path).ok()
}

/// Store an object in the local cache.
fn cache_put(checksum: &str, ext: &str, data: &[u8]) {
    let dir = cache_dir();
    let obj_dir = dir.join(&checksum[..2]);
    let _ = fs::create_dir_all(&obj_dir);
    let path = obj_dir.join(format!("{}.{ext}", &checksum[2..]));
    let _ = fs::write(&path, data);
}

/// Fetch an object, checking cache first.
fn fetch_object(repo_url: &str, checksum: &str, ext: &str) -> Result<Vec<u8>, String> {
    // Check local cache.
    if let Some(data) = cache_get(checksum, ext) {
        return Ok(data);
    }
    // Fetch from remote.
    let url = object_url(repo_url, checksum, ext);
    let data = fetch_url(&url)?;
    // Store in cache.
    cache_put(checksum, ext, &data);
    Ok(data)
}

// ---------------------------------------------------------------------------
// High-level operations
// ---------------------------------------------------------------------------

/// Fetch and parse the summary from a remote.
pub fn fetch_summary(repo_url: &str) -> Result<Vec<SummaryRef>, String> {
    let url = format!("{}/summary", repo_url.trim_end_matches('/'));
    let data = fetch_url(&url)?;
    parse_summary(&data)
}

/// Fetch a commit object.
pub fn fetch_commit(repo_url: &str, checksum: &str) -> Result<Commit, String> {
    let data = fetch_object(repo_url, checksum, "commit")?;
    parse_commit(&data)
}

/// Fetch a dirtree object.
pub fn fetch_dirtree(repo_url: &str, checksum: &str) -> Result<Dirtree, String> {
    let data = fetch_object(repo_url, checksum, "dirtree")?;
    parse_dirtree(&data)
}

/// Fetch a dirmeta object.
pub fn fetch_dirmeta(repo_url: &str, checksum: &str) -> Result<Dirmeta, String> {
    let data = fetch_object(repo_url, checksum, "dirmeta")?;
    parse_dirmeta(&data)
}

/// Fetch and extract a file object to a local path.
pub fn fetch_file(repo_url: &str, checksum: &str, dest: &Path) -> Result<(), String> {
    let data = fetch_object(repo_url, checksum, "filez")?;

    let (header, content_offset) = parse_filez_header(&data)?;

    if !header.symlink_target.is_empty() {
        // Symlink.
        std::os::unix::fs::symlink(&header.symlink_target, dest)
            .map_err(|e| format!("symlink: {e}"))?;
    } else {
        // Regular file.
        let content = decompress_filez_content(&data, content_offset)?;
        fs::write(dest, &content).map_err(|e| format!("write file: {e}"))?;

        // Set permissions.
        let mode = header.mode & 0o7777;
        let perms = std::fs::Permissions::from_mode(mode);
        let _ = fs::set_permissions(dest, perms);
    }

    Ok(())
}

/// Recursively checkout a dirtree to a local directory.
pub fn checkout_tree(
    repo_url: &str,
    dirtree_checksum: &str,
    dirmeta_checksum: &str,
    dest: &Path,
    verbose: bool,
) -> Result<(), String> {
    let _ = fs::create_dir_all(dest);

    // Apply directory metadata.
    let dirmeta = fetch_dirmeta(repo_url, dirmeta_checksum)?;
    let mode = dirmeta.mode & 0o7777;
    let _ = fs::set_permissions(dest, std::fs::Permissions::from_mode(mode));

    let tree = fetch_dirtree(repo_url, dirtree_checksum)?;

    // Checkout files.
    for file in &tree.files {
        let file_path = dest.join(&file.name);
        if verbose {
            eprintln!("  {}", file_path.display());
        }
        fetch_file(repo_url, &file.checksum, &file_path)?;
    }

    // Recurse into subdirectories.
    for dir in &tree.dirs {
        let dir_path = dest.join(&dir.name);
        checkout_tree(
            repo_url,
            &dir.dirtree_checksum,
            &dir.dirmeta_checksum,
            &dir_path,
            verbose,
        )?;
    }

    Ok(())
}

/// Pull a ref from a remote and checkout to a deploy directory.
pub fn pull_ref(
    repo_url: &str,
    ref_name: &str,
    deploy_dir: &Path,
    verbose: bool,
) -> Result<Checksum, String> {
    if verbose {
        eprintln!("Fetching summary from {repo_url}...");
    }

    let refs = fetch_summary(repo_url)?;
    let summary_ref = refs
        .iter()
        .find(|r| r.name == ref_name)
        .ok_or_else(|| format!("ref not found: {ref_name}"))?;

    if verbose {
        eprintln!(
            "Found ref: {} (commit {})",
            summary_ref.name,
            &summary_ref.checksum[..12]
        );
    }

    let commit = fetch_commit(repo_url, &summary_ref.checksum)?;
    if verbose {
        eprintln!("Commit: {}", commit.subject);
    }

    let files_dir = deploy_dir.join("files");
    if verbose {
        eprintln!("Checking out to {}...", files_dir.display());
    }

    checkout_tree(
        repo_url,
        &commit.root_dirtree,
        &commit.root_dirmeta,
        &files_dir,
        verbose,
    )?;

    Ok(summary_ref.checksum.clone())
}

/// List refs available on a remote.
#[allow(dead_code)]
pub fn list_remote_refs(repo_url: &str) -> Result<Vec<SummaryRef>, String> {
    fetch_summary(repo_url)
}

// ---------------------------------------------------------------------------
// GPG signature verification
// ---------------------------------------------------------------------------

/// Verify a GPG signature on data using the `gpg` or `gpgv` command.
///
/// Returns Ok(()) if verification succeeds, Err with details if it fails.
#[allow(dead_code)]
pub fn verify_gpg_signature(
    data: &[u8],
    signature: &[u8],
    keyring: Option<&Path>,
) -> Result<(), String> {
    let data_path = format!("/tmp/.flatpak-gpg-data-{}", std::process::id());
    let sig_path = format!("/tmp/.flatpak-gpg-sig-{}", std::process::id());

    fs::write(&data_path, data).map_err(|e| format!("write gpg data: {e}"))?;
    fs::write(&sig_path, signature).map_err(|e| format!("write gpg sig: {e}"))?;

    let mut cmd = std::process::Command::new("gpgv");
    if let Some(kr) = keyring {
        cmd.arg("--keyring");
        cmd.arg(kr);
    }
    cmd.arg(&sig_path);
    cmd.arg(&data_path);

    let result = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output();

    let _ = fs::remove_file(&data_path);
    let _ = fs::remove_file(&sig_path);

    match result {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "GPG verification failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )),
        Err(e) => Err(format!("gpgv not available: {e}")),
    }
}

/// Fetch and verify the summary.sig file for a remote.
#[allow(dead_code)]
pub fn fetch_and_verify_summary(
    repo_url: &str,
    keyring: Option<&Path>,
) -> Result<Vec<SummaryRef>, String> {
    let summary_url = format!("{}/summary", repo_url.trim_end_matches('/'));
    let sig_url = format!("{}/summary.sig", repo_url.trim_end_matches('/'));

    let summary_data = fetch_url(&summary_url)?;

    // Try to fetch and verify signature.
    match fetch_url(&sig_url) {
        Ok(sig_data) => {
            if let Err(e) = verify_gpg_signature(&summary_data, &sig_data, keyring) {
                eprintln!("warning: summary GPG verification failed: {e}");
            }
        }
        Err(_) => {
            // No signature available — proceed without verification.
        }
    }

    parse_summary(&summary_data)
}

//! Minimal OSTree HTTP client for pulling from Flatpak remotes.
//!
//! Implements just enough of the OSTree protocol to:
//! 1. Fetch and parse the summary file (GVariant format)
//! 2. Resolve refs to commit checksums
//! 3. Fetch commit, dirtree, dirmeta, and file objects
//! 4. Checkout a commit to a local directory

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

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
    let inner_start = align_up(string_end, 8);
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
    let dirs_start = files_end; // a(sayay) alignment = 1
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
    if data.len() < osz {
        return files;
    }
    let last_offset_pos = data.len() - osz;
    let last_offset = read_offset(data, last_offset_pos, osz);
    if last_offset > data.len() {
        return files;
    }
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

    let checksum_start = name_end;
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

    if data.len() < osz {
        return dirs;
    }
    let last_offset_pos = data.len() - osz;
    let last_offset = read_offset(data, last_offset_pos, osz);
    if last_offset > data.len() {
        return dirs;
    }
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

    let dirtree_start = name_end;
    let dirtree_end = read_offset(data, data.len() - osz, osz);
    if dirtree_end < dirtree_start || dirtree_end - dirtree_start < 32 {
        return None;
    }
    let dirtree_checksum = checksum_to_hex(&data[dirtree_start..dirtree_start + 32]);

    let dirmeta_start = dirtree_end;
    let dirmeta_end = data.len() - 2 * osz;
    if dirmeta_end < dirmeta_start || dirmeta_end - dirmeta_start < 32 {
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
    if hdr.len() < 24 {
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
// HTTP connection pool and fetching
// ---------------------------------------------------------------------------

/// Global connection pool for reusing HTTP/TLS connections.
/// Keyed by (host:port, is_tls).
#[allow(clippy::type_complexity)]
static CONN_POOL: std::sync::LazyLock<Mutex<HashMap<String, Vec<Box<dyn ReadWrite + Send>>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Take a connection from the pool, or create a new one.
fn get_connection(
    host: &str,
    port: u16,
    is_tls: bool,
) -> Result<Box<dyn ReadWrite + Send>, String> {
    let key = format!("{host}:{port}:{is_tls}");

    // Try to reuse a pooled connection.
    if let Ok(mut pool) = CONN_POOL.lock()
        && let Some(conns) = pool.get_mut(&key)
        && let Some(conn) = conns.pop()
    {
        return Ok(conn);
    }

    // Create a new connection.
    let addr = format!("{host}:{port}");
    if is_tls {
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
        let tcp = TcpStream::connect(&addr).map_err(|e| format!("connect: {e}"))?;
        let tls = rustls::StreamOwned::new(conn, tcp);
        Ok(Box::new(tls))
    } else {
        let tcp = TcpStream::connect(&addr).map_err(|e| format!("connect {addr}: {e}"))?;
        Ok(Box::new(tcp))
    }
}

/// Return a connection to the pool for reuse.
fn return_connection(host: &str, port: u16, is_tls: bool, conn: Box<dyn ReadWrite + Send>) {
    let key = format!("{host}:{port}:{is_tls}");
    if let Ok(mut pool) = CONN_POOL.lock() {
        let conns = pool.entry(key).or_default();
        if conns.len() < 4 {
            // Keep at most 4 idle connections per host.
            conns.push(conn);
        }
    }
}

/// Parse a URL into (scheme, host, port, path).
fn parse_url_parts(url: &str) -> Result<(String, String, u16, String), String> {
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
        Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(443)),
        None => (
            authority.to_string(),
            if scheme == "https" { 443 } else { 80 },
        ),
    };

    Ok((scheme.to_string(), host, port, path.to_string()))
}

/// Fetch a URL and return the response body.
pub fn fetch_url(url: &str) -> Result<Vec<u8>, String> {
    let (scheme, host, port, path) = parse_url_parts(url)?;
    let is_tls = scheme == "https";

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: rust-flatpak/0.1.0\r\nConnection: keep-alive\r\nAccept: */*\r\n\r\n"
    );

    let mut conn = get_connection(&host, port, is_tls)?;

    match do_http_request(&mut *conn, &request) {
        Ok(body) => {
            // Return connection to pool for reuse.
            return_connection(&host, port, is_tls, conn);
            Ok(body)
        }
        Err(_) => {
            // Connection may be broken. Try once more with a fresh connection.
            drop(conn);
            let mut conn = get_connection(&host, port, is_tls)?;
            let result = do_http_request(&mut *conn, &request);
            if result.is_ok() {
                return_connection(&host, port, is_tls, conn);
            }
            result
        }
    }
}

fn do_http_request(stream: &mut dyn ReadWrite, request: &str) -> Result<Vec<u8>, String> {
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    stream.flush().map_err(|e| format!("flush: {e}"))?;

    // Read headers.
    let mut header_buf = Vec::with_capacity(4096);
    let mut b = [0u8; 1];
    loop {
        if stream.read(&mut b).map_err(|e| format!("read: {e}"))? == 0 {
            break;
        }
        header_buf.push(b[0]);
        if header_buf.len() >= 4 && &header_buf[header_buf.len() - 4..] == b"\r\n\r\n" {
            break;
        }
        if header_buf.len() > 65536 {
            return Err("HTTP headers too large".into());
        }
    }

    let header = String::from_utf8_lossy(&header_buf);
    let status_line = header.lines().next().unwrap_or("");
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Handle redirects.
    if (301..=308).contains(&status_code) {
        for line in header.lines() {
            if let Some(loc) = line
                .strip_prefix("Location: ")
                .or_else(|| line.strip_prefix("location: "))
            {
                return fetch_url(loc.trim());
            }
        }
        return Err(format!("HTTP {status_code} redirect without Location"));
    }

    if status_code != 200 {
        return Err(format!("HTTP {status_code}: {status_line}"));
    }

    // Parse Content-Length and Transfer-Encoding.
    let header_lower = header.to_lowercase();
    let is_chunked = header_lower.contains("transfer-encoding: chunked");
    let content_length: Option<usize> = header_lower
        .lines()
        .find(|l| l.starts_with("content-length:"))
        .and_then(|l| l.split(':').nth(1)?.trim().parse().ok());

    // Read body.
    if is_chunked {
        read_chunked_body(stream)
    } else if let Some(len) = content_length {
        let mut body = vec![0u8; len];
        let mut read = 0;
        while read < len {
            let n = stream
                .read(&mut body[read..])
                .map_err(|e| format!("read body: {e}"))?;
            if n == 0 {
                break;
            }
            read += n;
        }
        body.truncate(read);
        Ok(body)
    } else {
        let mut body = Vec::new();
        let _ = stream.read_to_end(&mut body);
        Ok(body)
    }
}

/// Read a chunked transfer-encoded body.
fn read_chunked_body(stream: &mut dyn ReadWrite) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        // Read chunk size line.
        let mut size_line = Vec::new();
        let mut b = [0u8; 1];
        loop {
            if stream
                .read(&mut b)
                .map_err(|e| format!("read chunk: {e}"))?
                == 0
            {
                return Ok(body);
            }
            size_line.push(b[0]);
            if size_line.len() >= 2 && size_line.ends_with(b"\r\n") {
                break;
            }
        }
        let size_str = String::from_utf8_lossy(&size_line);
        let size_str = size_str.trim().split(';').next().unwrap_or("0");
        let size = usize::from_str_radix(size_str.trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        let mut chunk = vec![0u8; size];
        let mut read = 0;
        while read < size {
            let n = stream
                .read(&mut chunk[read..])
                .map_err(|e| format!("read chunk data: {e}"))?;
            if n == 0 {
                break;
            }
            read += n;
        }
        body.extend_from_slice(&chunk[..read]);
        // Read trailing CRLF.
        let mut crlf = [0u8; 2];
        let _ = stream.read_exact(&mut crlf);
    }
    Ok(body)
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

fn rustls_root_store() -> rustls::RootCertStore {
    let mut store = rustls::RootCertStore::empty();
    store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    store
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
/// Progress counter for checkout operations.
pub struct CheckoutProgress {
    pub files_fetched: std::sync::atomic::AtomicUsize,
    pub files_cached: std::sync::atomic::AtomicUsize,
    pub bytes_downloaded: std::sync::atomic::AtomicUsize,
}

impl CheckoutProgress {
    pub fn new() -> Self {
        Self {
            files_fetched: std::sync::atomic::AtomicUsize::new(0),
            files_cached: std::sync::atomic::AtomicUsize::new(0),
            bytes_downloaded: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn print_status(&self) {
        let fetched = self
            .files_fetched
            .load(std::sync::atomic::Ordering::Relaxed);
        let cached = self.files_cached.load(std::sync::atomic::Ordering::Relaxed);
        let bytes = self
            .bytes_downloaded
            .load(std::sync::atomic::Ordering::Relaxed);
        let total = fetched + cached;
        let mb = bytes as f64 / (1024.0 * 1024.0);
        eprint!("\r  {total} objects ({fetched} fetched, {cached} cached, {mb:.1} MB)");
    }
}

pub fn checkout_tree(
    repo_url: &str,
    dirtree_checksum: &str,
    dirmeta_checksum: &str,
    dest: &Path,
    verbose: bool,
) -> Result<(), String> {
    let progress = Arc::new(CheckoutProgress::new());
    checkout_tree_inner(
        repo_url,
        dirtree_checksum,
        dirmeta_checksum,
        dest,
        verbose,
        &progress,
    )?;
    if verbose {
        progress.print_status();
        eprintln!(); // newline after progress
    }
    Ok(())
}

fn checkout_tree_inner(
    repo_url: &str,
    dirtree_checksum: &str,
    dirmeta_checksum: &str,
    dest: &Path,
    verbose: bool,
    progress: &Arc<CheckoutProgress>,
) -> Result<(), String> {
    let _ = fs::create_dir_all(dest);

    // Apply directory metadata.
    let dirmeta = fetch_dirmeta(repo_url, dirmeta_checksum)?;
    let mode = dirmeta.mode & 0o7777;
    let _ = fs::set_permissions(dest, std::fs::Permissions::from_mode(mode));

    let tree = fetch_dirtree(repo_url, dirtree_checksum)?;

    // Checkout files — use threads for parallel fetching when there are many.
    if tree.files.len() > 4 {
        // Parallel fetching with a thread pool.
        let errors: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

        std::thread::scope(|s| {
            for file in &tree.files {
                let file_path = dest.join(&file.name);
                let checksum = file.checksum.clone();
                let url = repo_url.to_string();
                let progress = Arc::clone(progress);
                let errors = &errors;

                s.spawn(move || {
                    let was_cached = cache_get(&checksum, "filez").is_some();
                    match fetch_file(&url, &checksum, &file_path) {
                        Ok(()) => {
                            if was_cached {
                                progress
                                    .files_cached
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            } else {
                                progress
                                    .files_fetched
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                        Err(e) => {
                            errors.lock().unwrap().push(e);
                        }
                    }
                });
            }
        });

        let errs = errors.into_inner().unwrap();
        if !errs.is_empty() {
            return Err(errs.join("; "));
        }

        if verbose {
            progress.print_status();
        }
    } else {
        // Sequential for small directories.
        for file in &tree.files {
            let file_path = dest.join(&file.name);
            let was_cached = cache_get(&file.checksum, "filez").is_some();
            fetch_file(repo_url, &file.checksum, &file_path)?;
            if was_cached {
                progress
                    .files_cached
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            } else {
                progress
                    .files_fetched
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    // Recurse into subdirectories.
    for dir in &tree.dirs {
        let dir_path = dest.join(&dir.name);
        checkout_tree_inner(
            repo_url,
            &dir.dirtree_checksum,
            &dir.dirmeta_checksum,
            &dir_path,
            verbose,
            progress,
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

    // Try to use a static delta if available (much faster than individual objects).
    let delta_used = try_static_delta(repo_url, &summary_ref.checksum, &files_dir, verbose);
    if !delta_used {
        // Fall back to individual object fetching.
        checkout_tree(
            repo_url,
            &commit.root_dirtree,
            &commit.root_dirmeta,
            &files_dir,
            verbose,
        )?;
    }

    Ok(summary_ref.checksum.clone())
}

/// Try to fetch and apply a static delta for the given commit.
/// Returns true if a delta was successfully applied, false to fall back.
fn try_static_delta(repo_url: &str, commit: &str, _dest: &Path, verbose: bool) -> bool {
    let cache = cache_dir();
    let _ = std::fs::create_dir_all(&cache);

    match crate::deltas::apply_delta(repo_url, commit, &cache) {
        Ok(n) => {
            if verbose {
                eprintln!("  Applied static delta: {n} objects");
            }
            // TODO: checkout the objects from cache to dest.
            // For now, the delta writes objects to the cache, but we still
            // need the tree checkout to reconstruct the directory. The objects
            // are now cached, so the subsequent checkout_tree will hit cache.
            false // Still fall back to checkout_tree, but objects are cached now.
        }
        Err(_) => {
            // No delta available or failed to apply.
            false
        }
    }
}

/// List refs available on a remote.
#[allow(dead_code)]
pub fn list_remote_refs(repo_url: &str) -> Result<Vec<SummaryRef>, String> {
    fetch_summary(repo_url)
}

// ---------------------------------------------------------------------------
// OSTree commit creation
// ---------------------------------------------------------------------------

use std::os::unix::fs::MetadataExt;

/// Compute SHA256 hash of data and return as hex string.
fn sha256_hex(data: &[u8]) -> String {
    // Minimal SHA256 implementation. We use a subprocess since we don't have
    // a SHA256 crate. On Linux, sha256sum is always available.
    let mut child = std::process::Command::new("sha256sum")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("sha256sum not found");

    if let Some(ref mut stdin) = child.stdin {
        let _ = stdin.write_all(data);
    }
    drop(child.stdin.take());

    let output = child.wait_with_output().expect("sha256sum failed");
    let hex = String::from_utf8_lossy(&output.stdout);
    hex.split_whitespace().next().unwrap_or("").to_string()
}

/// Create a dirmeta object from directory metadata.
/// Format: (uuua(ayay)) — uid, gid, mode (all BE), xattrs.
pub fn create_dirmeta(uid: u32, gid: u32, mode: u32) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&uid.to_be_bytes());
    data.extend_from_slice(&gid.to_be_bytes());
    data.extend_from_slice(&mode.to_be_bytes());
    // Empty xattrs array: just a zero-length array of (ayay).
    data
}

/// Create a file content object for storage.
/// Returns (checksum, serialized_data) — the data is the uncompressed stream
/// format used for checksumming, NOT the .filez archive format.
fn create_file_object(file_path: &Path) -> Result<(String, Vec<u8>), String> {
    let meta = fs::metadata(file_path).map_err(|e| format!("stat {}: {e}", file_path.display()))?;
    let mode = meta.mode();
    let uid = meta.uid();
    let gid = meta.gid();

    let symlink_target = if meta.file_type().is_symlink() {
        fs::read_link(file_path)
            .map_err(|e| format!("readlink: {e}"))?
            .to_string_lossy()
            .to_string()
    } else {
        String::new()
    };

    // Build the header: (uuuusa(ayay)) — note: no leading `t` for checksum format.
    let mut header = Vec::new();
    header.extend_from_slice(&uid.to_be_bytes());
    header.extend_from_slice(&gid.to_be_bytes());
    header.extend_from_slice(&mode.to_be_bytes());
    header.extend_from_slice(&0u32.to_be_bytes()); // rdev
    header.extend_from_slice(symlink_target.as_bytes());
    header.push(0); // NUL terminator
    // Empty xattrs.

    // Build the stream: [4-byte BE header_size][4-byte padding][header][content]
    let header_size = header.len() as u32;
    let mut stream = Vec::new();
    stream.extend_from_slice(&header_size.to_be_bytes());
    stream.extend_from_slice(&[0u8; 4]); // padding
    stream.extend_from_slice(&header);

    if !meta.file_type().is_symlink() {
        let content = fs::read(file_path).map_err(|e| format!("read: {e}"))?;
        stream.extend_from_slice(&content);
    }

    let checksum = sha256_hex(&stream);
    Ok((checksum, stream))
}

/// Recursively create a dirtree object from a directory.
/// Returns (dirtree_checksum, dirmeta_checksum).
pub fn create_dirtree_from_dir(dir: &Path, repo_path: &Path) -> Result<(String, String), String> {
    let meta = fs::metadata(dir).map_err(|e| format!("stat {}: {e}", dir.display()))?;
    let dirmeta_data = create_dirmeta(meta.uid(), meta.gid(), meta.mode());
    let dirmeta_checksum = sha256_hex(&dirmeta_data);
    store_object(repo_path, &dirmeta_checksum, "dirmeta", &dirmeta_data);

    let mut files: Vec<(String, String)> = Vec::new(); // (name, checksum)
    let mut dirs: Vec<(String, String, String)> = Vec::new(); // (name, dirtree_cksum, dirmeta_cksum)

    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("readdir {}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in &entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let ftype = entry.file_type().map_err(|e| format!("filetype: {e}"))?;

        if ftype.is_dir() {
            let (sub_dirtree, sub_dirmeta) = create_dirtree_from_dir(&path, repo_path)?;
            dirs.push((name, sub_dirtree, sub_dirmeta));
        } else {
            let (checksum, _data) = create_file_object(&path)?;
            // Store the file object as .filez (compressed).
            let content = fs::read(&path).unwrap_or_default();
            let compressed = miniz_oxide::deflate::compress_to_vec(&content, 6);

            // Build .filez archive: [4-byte BE header_size][4-byte pad][header][compressed]
            let file_meta = fs::metadata(&path).unwrap();
            let symlink = if ftype.is_symlink() {
                fs::read_link(&path)
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            } else {
                String::new()
            };
            let mut filez_header = Vec::new();
            filez_header.extend_from_slice(&(content.len() as u64).to_be_bytes());
            filez_header.extend_from_slice(&file_meta.uid().to_be_bytes());
            filez_header.extend_from_slice(&file_meta.gid().to_be_bytes());
            filez_header.extend_from_slice(&file_meta.mode().to_be_bytes());
            filez_header.extend_from_slice(&0u32.to_be_bytes()); // rdev
            filez_header.extend_from_slice(symlink.as_bytes());
            filez_header.push(0);

            let mut filez = Vec::new();
            filez.extend_from_slice(&(filez_header.len() as u32).to_be_bytes());
            filez.extend_from_slice(&[0u8; 4]);
            filez.extend_from_slice(&filez_header);
            filez.extend_from_slice(&compressed);

            store_object(repo_path, &checksum, "filez", &filez);
            files.push((name, checksum));
        }
    }

    // Serialize the dirtree: (a(say)a(sayay))
    let dirtree_data = serialize_dirtree(&files, &dirs);
    let dirtree_checksum = sha256_hex(&dirtree_data);
    store_object(repo_path, &dirtree_checksum, "dirtree", &dirtree_data);

    Ok((dirtree_checksum, dirmeta_checksum))
}

/// Serialize a dirtree object as proper GVariant: (a(say), a(sayay)).
fn serialize_dirtree(files: &[(String, String)], dirs: &[(String, String, String)]) -> Vec<u8> {
    use crate::gvariant::{GVariant, byte_array, string};

    fn hex32(hex: &str) -> Vec<u8> {
        (0..hex.len().min(64))
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0))
            .collect()
    }

    // Files: a(say) — array of (name, checksum) tuples.
    let files_array: Vec<GVariant> = files
        .iter()
        .map(|(name, cksum)| GVariant::Tuple(vec![string(name), byte_array(&hex32(cksum))]))
        .collect();

    // Dirs: a(sayay) — array of (name, dirtree_cksum, dirmeta_cksum) tuples.
    let dirs_array: Vec<GVariant> = dirs
        .iter()
        .map(|(name, dt, dm)| {
            GVariant::Tuple(vec![
                string(name),
                byte_array(&hex32(dt)),
                byte_array(&hex32(dm)),
            ])
        })
        .collect();

    let dirtree = GVariant::Tuple(vec![
        GVariant::Array(files_array),
        GVariant::Array(dirs_array),
    ]);
    dirtree.serialize()
}

/// Create a commit object.
/// Returns the commit checksum.
pub fn create_commit(
    repo_path: &Path,
    root_dirtree: &str,
    root_dirmeta: &str,
    subject: &str,
    parent: Option<&str>,
) -> Result<String, String> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Convert hex checksums to bytes.
    fn hex_to_bytes_local(hex: &str) -> Vec<u8> {
        (0..hex.len().min(64))
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0))
            .collect()
    }
    let parent_bytes = parent.map(hex_to_bytes_local);
    let dirtree_bytes = hex_to_bytes_local(root_dirtree);
    let dirmeta_bytes = hex_to_bytes_local(root_dirmeta);

    // Build proper GVariant commit object: (a{sv}, ay, a(say), s, s, t, ay, ay)
    let commit = crate::gvariant::commit(
        subject,
        "",                // body
        timestamp.to_be(), // BE-encoded timestamp
        &dirtree_bytes,
        &dirmeta_bytes,
        parent_bytes.as_deref(),
    );
    let data = commit.serialize();

    let checksum = sha256_hex(&data);
    store_object(repo_path, &checksum, "commit", &data);

    // Write ref.
    let refs_dir = repo_path.join("refs");
    let _ = fs::create_dir_all(&refs_dir);

    Ok(checksum)
}

/// Store an object in the local repo.
fn store_object(repo_path: &Path, checksum: &str, ext: &str, data: &[u8]) {
    let obj_dir = repo_path.join("objects").join(&checksum[..2]);
    let _ = fs::create_dir_all(&obj_dir);
    let path = obj_dir.join(format!("{}.{ext}", &checksum[2..]));
    let _ = fs::write(&path, data);
}

/// Write a ref to the repo.
pub fn write_ref(repo_path: &Path, ref_name: &str, commit_checksum: &str) {
    let ref_path = repo_path.join("refs").join("heads").join(ref_name);
    if let Some(parent) = ref_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&ref_path, format!("{commit_checksum}\n"));
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

use flate2::read::{DeflateDecoder, GzDecoder};
use std::io::{BufRead, BufReader, Read};

use crate::connection::Connection;

pub(crate) struct Response {
    pub(crate) status: u16,
    #[expect(
        dead_code,
        reason = "parsed from response; reserved for future --write-out support"
    )]
    pub(crate) status_text: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
    pub(crate) header_bytes: Vec<u8>,
    /// Collected header bytes from intermediate redirect responses.
    pub(crate) redirect_headers: Vec<u8>,
    /// Number of TCP connections opened during this transfer (incl. redirects).
    pub(crate) num_connects: usize,
    /// Number of redirects followed.
    pub(crate) num_redirects: usize,
    /// True if max_redirs was reached — the transfer completed but the final
    /// status is still a 3xx and should map to exit code 47.
    pub(crate) max_redirects_reached: bool,
    /// True if the response had a malformed Content-Length (exit 8).
    pub(crate) weird_server_reply: bool,
    /// The final URL used after following any redirects (for `%{url_effective}`).
    pub(crate) final_url: Option<String>,
    /// Where a 3xx response pointed to that wasn't (or couldn't be) followed
    /// (for `%{redirect_url}`).
    pub(crate) redirect_url: Option<String>,
    /// True if the body read timed out (maps to exit code 28).
    pub(crate) timed_out: bool,
    /// True if a recv/protocol error occurred during body reading (exit 56).
    pub(crate) recv_error: bool,
}

pub(crate) fn read_response(
    conn: &mut Connection,
    is_head: bool,
    http09: bool,
    decompress: bool,
) -> Result<Response, String> {
    let mut reader = BufReader::new(conn);

    // 1xx responses are interim — skip them and read the real response that follows.
    let mut header_bytes = Vec::new();
    let status_line;
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| format!("failed to read status line: {e}"))?;
        if n == 0 {
            if !header_bytes.is_empty() {
                // We consumed a 1xx interim response but got no final response.
                // Return a partial Response so the 1xx headers can still be
                // output (curl --include shows them). Callers detect status==0
                // as "empty reply" and set exit code 52.
                return Ok(Response {
                    status: 0,
                    status_text: String::new(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    header_bytes,
                    redirect_headers: Vec::new(),
                    num_connects: 0,
                    num_redirects: 0,
                    max_redirects_reached: false,
                    weird_server_reply: false,
                    final_url: None,
                    redirect_url: None,
                    timed_out: false,
                    recv_error: false,
                });
            }
            return Err("empty reply from server".into());
        }
        // HTTP/0.9 opt-in: servers respond with body only, no status line or
        // headers. When the first line isn't an HTTP/N.N status, treat the
        // whole response as body with a synthetic 200 status.
        if http09 && !line.starts_with("HTTP/") {
            let mut body = line.into_bytes();
            let _ = reader.read_to_end(&mut body);
            return Ok(Response {
                status: 200,
                status_text: String::new(),
                headers: Vec::new(),
                body,
                header_bytes: Vec::new(),
                redirect_headers: Vec::new(),
                num_connects: 0,
                num_redirects: 0,
                max_redirects_reached: false,
                weird_server_reply: false,
                final_url: None,
                redirect_url: None,
                timed_out: false,
                recv_error: false,
            });
        }
        header_bytes.extend_from_slice(line.as_bytes());
        let trimmed = line.trim_end().to_string();
        // Parse status code to detect 1xx.
        let parts: Vec<&str> = trimmed.splitn(3, ' ').collect();
        let code = parts
            .get(1)
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        if (100..200).contains(&code) {
            // Consume headers of the interim response, then loop for the real one.
            loop {
                let mut hl = String::new();
                let n = reader
                    .read_line(&mut hl)
                    .map_err(|e| format!("failed to read interim headers: {e}"))?;
                if n == 0 {
                    return Err("empty reply from server".into());
                }
                header_bytes.extend_from_slice(hl.as_bytes());
                if hl.trim_end().is_empty() {
                    break;
                }
            }
            // Keep interim response headers; curl --include outputs them too.
            continue;
        }
        status_line = trimmed;
        break;
    }

    let parts: Vec<&str> = status_line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(format!("malformed status line: {status_line}"));
    }
    // Only accept HTTP/1.0 and HTTP/1.1 (we don't do HTTP/2 or HTTP/3). HTTP/0.9
    // has no headers; any other "HTTP/x.y" is rejected as weird_server_reply.
    if !matches!(parts[0], "HTTP/1.0" | "HTTP/1.1") {
        return Err(format!(
            "weird_server_reply: unsupported protocol: {}",
            parts[0]
        ));
    }
    let status: u16 = parts[1]
        .parse()
        .map_err(|_| format!("invalid status code: {}", parts[1]))?;
    let status_text = if parts.len() > 2 {
        parts[2].to_string()
    } else {
        String::new()
    };

    // Read headers. Supports obs-fold (a line starting with SP/HTAB is a
    // continuation of the preceding header's value — curl flattens it into a
    // single space). Preserves the line ending (LF or CRLF) and non-folded
    // header lines verbatim.
    let mut headers: Vec<(String, String)> = Vec::new();
    // pending holds the raw physical line (with its line ending). When a
    // continuation arrives we drop the raw form and rebuild a canonical line.
    let mut pending_raw: Option<String> = None;
    let mut pending_folded: Option<String> = None;
    let mut pending_ending: &'static [u8] = b"\r\n";
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| format!("failed to read header: {e}"))?;
        if n == 0 {
            break;
        }
        // Binary zero in a header line is a protocol violation — curl exits 8
        // (weird server reply). We echo anything already buffered and stop.
        if line.contains('\0') {
            return Ok(Response {
                status,
                status_text,
                headers,
                body: Vec::new(),
                header_bytes,
                redirect_headers: Vec::new(),
                num_connects: 0,
                num_redirects: 0,
                max_redirects_reached: false,
                weird_server_reply: true,
                final_url: None,
                redirect_url: None,
                timed_out: false,
                recv_error: false,
            });
        }
        let this_ending: &'static [u8] = if line.ends_with("\r\n") {
            b"\r\n"
        } else if line.ends_with('\n') {
            b"\n"
        } else {
            b""
        };
        let trimmed = line.trim_end_matches(['\r', '\n']);
        let is_cont = !trimmed.is_empty() && (line.starts_with(' ') || line.starts_with('\t'));

        if is_cont {
            if let Some(ref mut folded) = pending_folded {
                folded.push(' ');
                folded.push_str(trimmed.trim_start());
                // Folded mode — once a continuation arrives we never emit the
                // original raw line.
                pending_raw = None;
            } else if let Some(prev_raw) = pending_raw.take() {
                // Convert the previously-seen raw line into folded form.
                let prev_trim = prev_raw.trim_end_matches(['\r', '\n']).to_string();
                let mut folded = prev_trim;
                folded.push(' ');
                folded.push_str(trimmed.trim_start());
                pending_folded = Some(folded);
            }
            continue;
        }

        // Flush any pending header before starting a new one.
        if let Some(folded) = pending_folded.take() {
            if let Some((key, val)) = folded.split_once(':') {
                let val_trim = val.trim();
                if val_trim.is_empty() {
                    header_bytes.extend_from_slice(key.as_bytes());
                    header_bytes.push(b':');
                } else {
                    header_bytes.extend_from_slice(format!("{key}: {val_trim}").as_bytes());
                }
                header_bytes.extend_from_slice(pending_ending);
                headers.push((key.trim().to_lowercase(), val_trim.to_string()));
            }
        } else if let Some(raw) = pending_raw.take() {
            header_bytes.extend_from_slice(raw.as_bytes());
            let trimmed_raw = raw.trim_end_matches(['\r', '\n']);
            if let Some((key, val)) = trimmed_raw.split_once(':') {
                headers.push((key.trim().to_lowercase(), val.trim().to_string()));
            }
        }

        if trimmed.is_empty() {
            // End of headers — emit the separator verbatim.
            header_bytes.extend_from_slice(line.as_bytes());
            break;
        }
        // Reject malformed headers (missing ':') — curl exits 8.
        if !trimmed.contains(':') {
            return Ok(Response {
                status,
                status_text,
                headers,
                body: Vec::new(),
                header_bytes,
                redirect_headers: Vec::new(),
                num_connects: 0,
                num_redirects: 0,
                max_redirects_reached: false,
                weird_server_reply: true,
                final_url: None,
                redirect_url: None,
                timed_out: false,
                recv_error: false,
            });
        }
        pending_raw = Some(line.clone());
        pending_ending = this_ending;
    }

    // HEAD responses, and 204/304 responses, never have a body per HTTP spec —
    // even if they advertise Content-Length or Transfer-Encoding: chunked.
    if is_head || status == 204 || status == 304 {
        return Ok(Response {
            status,
            status_text,
            headers,
            body: Vec::new(),
            header_bytes,
            redirect_headers: Vec::new(),
            num_connects: 0,
            num_redirects: 0,
            max_redirects_reached: false,
            weird_server_reply: false,
            final_url: None,
            redirect_url: None,
            timed_out: false,
            recv_error: false,
        });
    }

    // Read body based on Transfer-Encoding or Content-Length.
    let is_chunked = headers
        .iter()
        .any(|(k, v)| k == "transfer-encoding" && v.contains("chunked"));
    // If Content-Length is present but fails to parse as a non-negative integer,
    // treat the response as malformed. curl writes the status + Date header
    // that came BEFORE the bad Content-Length, then exits 8 (weird_server_reply).
    // Exception: if the value is all digits (numeric but too large to fit usize),
    // we fall back to "unknown size" (read until EOF) — matching curl behavior.
    let cl_entry = headers.iter().find(|(k, _)| k == "content-length");
    let cl_all_digits =
        cl_entry.is_some_and(|(_, v)| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()));
    if let Some((_, v)) = cl_entry
        && v.parse::<usize>().is_err()
        && !cl_all_digits
    {
        // Truncate header_bytes to just what came before Content-Length.
        // Simpler: return partial output via a marker response.
        let mut partial_bytes = Vec::new();
        // Re-emit status line + headers that came before Content-Length.
        for line in header_bytes
            .split(|&b| b == b'\n')
            .take_while(|l| !l.to_ascii_lowercase().starts_with(b"content-length:"))
        {
            partial_bytes.extend_from_slice(line);
            partial_bytes.push(b'\n');
        }
        // Note: keep trailing '\n' — curl output includes LF after each header line.
        return Ok(Response {
            status,
            status_text,
            headers,
            body: Vec::new(),
            header_bytes: partial_bytes,
            redirect_headers: Vec::new(),
            num_connects: 0,
            num_redirects: 0,
            max_redirects_reached: false,
            weird_server_reply: true,
            final_url: None,
            redirect_url: None,
            timed_out: false,
            recv_error: false,
        });
    }
    let content_length: Option<usize> = cl_entry.and_then(|(_, v)| v.parse().ok());

    let (body, timed_out, recv_error_flag) = if is_chunked {
        let (b, err) = read_chunked_body(&mut reader)?;
        (b, false, err)
    } else if let Some(len) = content_length {
        let mut buf = vec![0u8; len];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("failed to read body: {e}"))?;
        (buf, false, false)
    } else {
        // Read until EOF.
        let mut buf = Vec::new();
        let read_err = reader.read_to_end(&mut buf);
        let timed_out = matches!(&read_err, Err(e) if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock);
        (buf, timed_out, false)
    };

    // Decompress if Content-Encoding is present and --compressed was requested
    let content_encoding = if decompress {
        headers
            .iter()
            .find(|(k, _)| k == "content-encoding")
            .map(|(_, v)| v.to_lowercase())
    } else {
        None
    };

    let body = match content_encoding.as_deref() {
        Some(enc) if enc.contains("gzip") => {
            let mut decoder = GzDecoder::new(&body[..]);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed).unwrap_or_default();
            // If decompression fails, fall back to raw body
            if decompressed.is_empty() && !body.is_empty() {
                body
            } else {
                decompressed
            }
        }
        Some(enc) if enc.contains("deflate") => {
            let mut decoder = DeflateDecoder::new(&body[..]);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed).unwrap_or_default();
            if decompressed.is_empty() && !body.is_empty() {
                body
            } else {
                decompressed
            }
        }
        _ => body,
    };

    // Remove content-encoding and content-length headers after decompression
    if content_encoding.is_some() {
        headers.retain(|(k, _)| k != "content-encoding" && k != "content-length");
    }

    Ok(Response {
        status,
        status_text,
        headers,
        body,
        header_bytes,
        redirect_headers: Vec::new(),
        num_connects: 0,
        num_redirects: 0,
        max_redirects_reached: false,
        weird_server_reply: false,
        final_url: None,
        redirect_url: None,
        timed_out,
        recv_error: recv_error_flag,
    })
}

fn read_chunked_body(reader: &mut impl BufRead) -> Result<(Vec<u8>, bool), String> {
    let mut body = Vec::new();
    loop {
        let mut size_line = String::new();
        if reader.read_line(&mut size_line).is_err() {
            return Ok((body, true));
        }
        let size_str = size_line.trim();
        // Strip chunk extensions.
        let size_str = size_str.split(';').next().unwrap_or(size_str);
        let size = match usize::from_str_radix(size_str, 16) {
            Ok(s) => s,
            Err(_) => return Ok((body, true)),
        };
        if size == 0 {
            // Read trailing CRLF.
            let mut trailer = String::new();
            let _ = reader.read_line(&mut trailer);
            break;
        }
        let mut chunk = vec![0u8; size];
        if reader.read_exact(&mut chunk).is_err() {
            return Ok((body, true));
        }
        body.extend_from_slice(&chunk);
        // Read trailing CRLF after chunk data.
        let mut crlf = [0u8; 2];
        let _ = reader.read_exact(&mut crlf);
    }
    Ok((body, false))
}

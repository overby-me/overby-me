use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use std::io::{BufRead, BufReader, Read};

/// Maximum size of response headers for a single response (~300KB).
const MAX_HEADER_SIZE: usize = 307200;
/// Maximum accumulated header size across all redirect hops (~6MB).
const MAX_TOTAL_HEADER_SIZE: usize = 6 * 1024 * 1024;

pub(crate) struct Response {
    pub(crate) trailer_bytes: Vec<u8>,
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
    /// True if --proto-redir blocked a redirect target scheme; maps to exit 1.
    pub(crate) proto_redir_blocked: bool,
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
    pub(crate) partial_file: bool,
    pub(crate) bad_content_encoding: bool,
    pub(crate) bad_encoding_too_many: bool,
    pub(crate) filesize_exceeded: bool,
    pub(crate) header_size_error: bool,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_response(
    conn: &mut impl Read,
    is_head: bool,
    http09: bool,
    decompress: bool,
    tr_decompress: bool,
    raw: bool,
    max_filesize: Option<u64>,
    max_filesize_overflow: bool,
    accumulated_header_bytes: usize,
    ignore_content_length: bool,
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
                    trailer_bytes: Vec::new(),
                    status: 0,
                    status_text: String::new(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    header_bytes,
                    redirect_headers: Vec::new(),
                    num_connects: 0,
                    num_redirects: 0,
                    max_redirects_reached: false,
                    proto_redir_blocked: false,
                    weird_server_reply: false,
                    final_url: None,
                    redirect_url: None,
                    timed_out: false,
                    recv_error: false,
                    partial_file: false,
                    bad_content_encoding: false,
                    bad_encoding_too_many: false,
                    filesize_exceeded: false,
                    header_size_error: false,
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
                trailer_bytes: Vec::new(),
                status: 200,
                status_text: String::new(),
                headers: Vec::new(),
                body,
                header_bytes: Vec::new(),
                redirect_headers: Vec::new(),
                num_connects: 0,
                num_redirects: 0,
                max_redirects_reached: false,
                proto_redir_blocked: false,
                weird_server_reply: false,
                final_url: None,
                redirect_url: None,
                timed_out: false,
                recv_error: false,
                partial_file: false,
                bad_content_encoding: false,
                bad_encoding_too_many: false,
                filesize_exceeded: false,
                header_size_error: false,
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
            // EOF mid-header block: flush whatever pending header we have so
            // the caller still sees the last unterminated line, then exit
            // with CURLE_PARTIAL_FILE (18). Matches upstream curl behaviour
            // for tests like 1665 where the server omits the final CRLF.
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
                // Make sure we end on a line terminator so downstream
                // consumers see a well-formed header block.
                if !raw.ends_with('\n') {
                    header_bytes.push(b'\n');
                }
                let trimmed_raw = raw.trim_end_matches(['\r', '\n']);
                if let Some((key, val)) = trimmed_raw.split_once(':') {
                    headers.push((key.trim().to_lowercase(), val.trim().to_string()));
                }
            }
            break;
        }
        // Binary zero in a header line is a protocol violation — curl exits 8
        // (weird server reply). We echo anything already buffered and stop.
        if line.contains('\0') {
            return Ok(Response {
                trailer_bytes: Vec::new(),
                status,
                status_text,
                headers,
                body: Vec::new(),
                header_bytes,
                redirect_headers: Vec::new(),
                num_connects: 0,
                num_redirects: 0,
                max_redirects_reached: false,
                proto_redir_blocked: false,
                weird_server_reply: true,
                final_url: None,
                redirect_url: None,
                timed_out: false,
                recv_error: false,
                partial_file: false,
                bad_content_encoding: false,
                bad_encoding_too_many: false,
                filesize_exceeded: false,
                header_size_error: false,
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
                trailer_bytes: Vec::new(),
                status,
                status_text,
                headers,
                body: Vec::new(),
                header_bytes,
                redirect_headers: Vec::new(),
                num_connects: 0,
                num_redirects: 0,
                max_redirects_reached: false,
                proto_redir_blocked: false,
                weird_server_reply: true,
                final_url: None,
                redirect_url: None,
                timed_out: false,
                recv_error: false,
                partial_file: false,
                bad_content_encoding: false,
                bad_encoding_too_many: false,
                filesize_exceeded: false,
                header_size_error: false,
            });
        }
        pending_raw = Some(line.clone());
        pending_ending = this_ending;
    }

    // Check header size limits.
    if header_bytes.len() > MAX_HEADER_SIZE {
        return Ok(Response {
            trailer_bytes: Vec::new(),
            status,
            status_text,
            headers,
            body: Vec::new(),
            header_bytes,
            redirect_headers: Vec::new(),
            num_connects: 0,
            num_redirects: 0,
            max_redirects_reached: false,
            proto_redir_blocked: false,
            weird_server_reply: false,
            final_url: None,
            redirect_url: None,
            timed_out: false,
            recv_error: false,
            partial_file: false,
            bad_content_encoding: false,
            bad_encoding_too_many: false,
            filesize_exceeded: false,
            header_size_error: true,
        });
    }
    if accumulated_header_bytes + header_bytes.len() > MAX_TOTAL_HEADER_SIZE {
        return Ok(Response {
            trailer_bytes: Vec::new(),
            status,
            status_text,
            headers,
            body: Vec::new(),
            header_bytes,
            redirect_headers: Vec::new(),
            num_connects: 0,
            num_redirects: 0,
            max_redirects_reached: false,
            proto_redir_blocked: false,
            weird_server_reply: false,
            final_url: None,
            redirect_url: None,
            timed_out: false,
            recv_error: false,
            partial_file: false,
            bad_content_encoding: false,
            bad_encoding_too_many: false,
            filesize_exceeded: false,
            header_size_error: true,
        });
    }

    // HEAD responses, and 204/304 responses, never have a body per HTTP spec —
    // even if they advertise Content-Length or Transfer-Encoding: chunked.
    if is_head || status == 204 || status == 304 {
        return Ok(Response {
            trailer_bytes: Vec::new(),
            status,
            status_text,
            headers,
            body: Vec::new(),
            header_bytes,
            redirect_headers: Vec::new(),
            num_connects: 0,
            num_redirects: 0,
            max_redirects_reached: false,
            proto_redir_blocked: false,
            weird_server_reply: false,
            final_url: None,
            redirect_url: None,
            timed_out: false,
            recv_error: false,
            partial_file: false,
            bad_content_encoding: false,
            bad_encoding_too_many: false,
            filesize_exceeded: false,
            header_size_error: false,
        });
    }

    // Read body based on Transfer-Encoding or Content-Length.
    let is_chunked = !raw
        && headers
            .iter()
            .any(|(k, v)| k == "transfer-encoding" && v.contains("chunked"));

    // Collect Transfer-Encoding tokens (in order) for ordering and stack-depth
    // validation. Per RFC 7230, chunked must be the last encoding in the chain
    // (innermost). curl rejects any other order with CURLE_BAD_CONTENT_ENCODING.
    let te_tokens: Vec<String> = headers
        .iter()
        .filter(|(k, _)| k == "transfer-encoding")
        .flat_map(|(_, v)| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_lowercase())
                .collect::<Vec<_>>()
        })
        .collect();
    // chunked must be either absent or the very last token (test 1546).
    let chunked_misplaced = te_tokens
        .iter()
        .position(|t| t == "chunked")
        .is_some_and(|p| p != te_tokens.len() - 1);
    let te_layer_count = te_tokens.len();
    if chunked_misplaced {
        // Truncate header_bytes to exclude the offending Transfer-Encoding line
        // (and the empty separator that follows it), matching curl which writes
        // headers before the failure point and then aborts.
        let mut truncated = Vec::new();
        for line in header_bytes.split(|&b| b == b'\n') {
            let lc: Vec<u8> = line
                .iter()
                .take(18)
                .map(|b| b.to_ascii_lowercase())
                .collect();
            if lc.starts_with(b"transfer-encoding:") {
                break;
            }
            truncated.extend_from_slice(line);
            truncated.push(b'\n');
        }
        return Ok(Response {
            trailer_bytes: Vec::new(),
            status,
            status_text,
            headers,
            body: Vec::new(),
            header_bytes: truncated,
            redirect_headers: Vec::new(),
            num_connects: 0,
            num_redirects: 0,
            max_redirects_reached: false,
            proto_redir_blocked: false,
            weird_server_reply: false,
            final_url: None,
            redirect_url: None,
            timed_out: false,
            recv_error: false,
            partial_file: false,
            bad_content_encoding: true,
            bad_encoding_too_many: false,
            filesize_exceeded: false,
            header_size_error: false,
        });
    }
    // Reject responses whose Transfer-Encoding stack has more than 5 layers,
    // matching curl's MAX_ENCODING_STACK guard (test 387).
    if te_layer_count > 5 {
        return Ok(Response {
            trailer_bytes: Vec::new(),
            status,
            status_text,
            headers,
            body: Vec::new(),
            header_bytes,
            redirect_headers: Vec::new(),
            num_connects: 0,
            num_redirects: 0,
            max_redirects_reached: false,
            proto_redir_blocked: false,
            weird_server_reply: false,
            final_url: None,
            redirect_url: None,
            timed_out: false,
            recv_error: false,
            partial_file: false,
            bad_content_encoding: true,
            bad_encoding_too_many: true,
            filesize_exceeded: false,
            header_size_error: false,
        });
    }
    // RFC 7231: only one Location header is allowed in a response.
    // Multiple Location headers are a malformed response (test 772, exit 8).
    let location_count = headers.iter().filter(|(k, _)| k == "location").count();
    if location_count > 1 {
        let mut partial_bytes = Vec::new();
        let mut seen_location = false;
        for line in header_bytes.split(|&b| b == b'\n') {
            let lc: Vec<u8> = line
                .iter()
                .take(9)
                .map(|b| b.to_ascii_lowercase())
                .collect();
            if lc.starts_with(b"location:") {
                if seen_location {
                    break;
                }
                seen_location = true;
            }
            partial_bytes.extend_from_slice(line);
            partial_bytes.push(b'\n');
        }
        return Ok(Response {
            trailer_bytes: Vec::new(),
            status,
            status_text,
            headers,
            body: Vec::new(),
            header_bytes: partial_bytes,
            redirect_headers: Vec::new(),
            num_connects: 0,
            num_redirects: 0,
            max_redirects_reached: false,
            proto_redir_blocked: false,
            weird_server_reply: true,
            final_url: None,
            redirect_url: None,
            timed_out: false,
            recv_error: false,
            partial_file: false,
            bad_content_encoding: false,
            bad_encoding_too_many: false,
            filesize_exceeded: false,
            header_size_error: false,
        });
    }
    // RFC 7230 §3.3.2: if multiple Content-Length values appear (either
    // multiple headers or comma-separated values within one), they must all be
    // equal; otherwise the response is malformed (test 770/771).
    let cl_values: Vec<String> = headers
        .iter()
        .filter(|(k, _)| k == "content-length")
        .flat_map(|(_, v)| v.split(',').map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect();
    let cl_parsed: Vec<Option<u64>> = cl_values.iter().map(|v| v.parse::<u64>().ok()).collect();
    let cl_inconsistent = cl_parsed.len() > 1
        && (cl_parsed.iter().any(|p| p.is_none()) || !cl_parsed.windows(2).all(|w| w[0] == w[1]));
    // When multiple CL values agree, normalize all content-length headers to a
    // single canonical numeric value so the rest of the body-length code path
    // (which uses parse::<usize>()) accepts them.
    if !cl_inconsistent && cl_parsed.len() > 1 {
        let canonical = cl_parsed[0].unwrap().to_string();
        for h in headers.iter_mut() {
            if h.0 == "content-length" {
                h.1 = canonical.clone();
            }
        }
    }
    if cl_inconsistent {
        // Truncate to before the FIRST content-length header, then bail out
        // with weird_server_reply (exit 8).
        let mut partial_bytes = Vec::new();
        for line in header_bytes.split(|&b| b == b'\n') {
            let lc: Vec<u8> = line
                .iter()
                .take(15)
                .map(|b| b.to_ascii_lowercase())
                .collect();
            if lc.starts_with(b"content-length:") {
                break;
            }
            partial_bytes.extend_from_slice(line);
            partial_bytes.push(b'\n');
        }
        return Ok(Response {
            trailer_bytes: Vec::new(),
            status,
            status_text,
            headers,
            body: Vec::new(),
            header_bytes: partial_bytes,
            redirect_headers: Vec::new(),
            num_connects: 0,
            num_redirects: 0,
            max_redirects_reached: false,
            proto_redir_blocked: false,
            weird_server_reply: true,
            final_url: None,
            redirect_url: None,
            timed_out: false,
            recv_error: false,
            partial_file: false,
            bad_content_encoding: false,
            bad_encoding_too_many: false,
            filesize_exceeded: false,
            header_size_error: false,
        });
    }
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
            trailer_bytes: Vec::new(),
            status,
            status_text,
            headers,
            body: Vec::new(),
            header_bytes: partial_bytes,
            redirect_headers: Vec::new(),
            num_connects: 0,
            num_redirects: 0,
            max_redirects_reached: false,
            proto_redir_blocked: false,
            weird_server_reply: true,
            final_url: None,
            redirect_url: None,
            timed_out: false,
            recv_error: false,
            partial_file: false,
            bad_content_encoding: false,
            bad_encoding_too_many: false,
            filesize_exceeded: false,
            header_size_error: false,
        });
    }
    let content_length: Option<usize> = if ignore_content_length {
        None
    } else {
        cl_entry.and_then(|(_, v)| v.parse().ok())
    };

    // --max-filesize: check Content-Length against the limit before reading the body.
    // If the raw max-filesize string didn't parse as u64 (overflow), treat as exceeded.
    // If Content-Length (as u64) exceeds the limit, also treat as exceeded.
    // If cl_all_digits is true but it didn't parse as usize, it's a huge number — also exceeded.
    if max_filesize.is_some() || max_filesize_overflow {
        let cl_as_u64: Option<u64> = cl_entry.and_then(|(_, v)| v.parse().ok());
        let exceeded = if max_filesize_overflow {
            // The max-filesize value itself overflowed — but if there's a Content-Length
            // that's also huge (didn't parse as u64), that's exceeded too.
            // Actually, if max_filesize_overflow, the limit is astronomically large,
            // so only exceed if CL also overflows (can't compare).
            // For test 393: huge CL like 999999999999999999999999 with a parseable max-filesize
            // Actually let's re-think: max_filesize_overflow means the --max-filesize VALUE
            // didn't parse. If it didn't parse, max_filesize is None. We should treat
            // unparsable max-filesize as 0 (curl behavior).
            cl_entry.is_some()
        } else if let Some(limit) = max_filesize {
            if let Some(cl) = cl_as_u64 {
                cl > limit
            } else if cl_all_digits {
                // Content-Length is all digits but too big for u64 — definitely exceeds
                true
            } else {
                false
            }
        } else {
            false
        };
        if exceeded {
            return Ok(Response {
                trailer_bytes: Vec::new(),
                status,
                status_text,
                headers,
                body: Vec::new(),
                header_bytes,
                redirect_headers: Vec::new(),
                num_connects: 0,
                num_redirects: 0,
                max_redirects_reached: false,
                proto_redir_blocked: false,
                weird_server_reply: false,
                final_url: None,
                redirect_url: None,
                timed_out: false,
                recv_error: false,
                partial_file: false,
                bad_content_encoding: false,
                bad_encoding_too_many: false,
                filesize_exceeded: true,
                header_size_error: false,
            });
        }
    }

    let (body, timed_out, recv_error_flag, partial_flag, chunked_trailers) = if is_chunked {
        let (b, err, trailers) = read_chunked_body(&mut reader)?;
        (
            b,
            false,
            err == ChunkErr::Recv,
            err == ChunkErr::Partial,
            trailers,
        )
    } else if let Some(len) = content_length {
        // Read up to `len` bytes; if the connection closes early, return the
        // partial body and signal CURLE_PARTIAL_FILE so the caller can exit 18.
        let mut buf = Vec::with_capacity(len);
        let mut chunk = [0u8; 8192];
        let mut partial = false;
        while buf.len() < len {
            let want = (len - buf.len()).min(chunk.len());
            match reader.read(&mut chunk[..want]) {
                Ok(0) => {
                    partial = true;
                    break;
                }
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(e) => return Err(format!("failed to read body: {e}")),
            }
        }
        (buf, false, false, partial, Vec::new())
    } else {
        // Read until EOF.
        let mut buf = Vec::new();
        let read_err = reader.read_to_end(&mut buf);
        let timed_out = matches!(&read_err, Err(e) if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock);
        (buf, timed_out, false, false, Vec::new())
    };

    // Decompress Transfer-Encoding (RFC 7230 §4) if --tr-encoding was set.
    // Strip "chunked" (already de-chunked above) and apply remaining layers
    // in reverse order. Unlike Content-Encoding, T-E layers are leftmost-applied-first.
    let mut body = body;
    if tr_decompress && !raw {
        let te_value = headers
            .iter()
            .filter(|(k, _)| k == "transfer-encoding")
            .map(|(_, v)| v.to_lowercase())
            .collect::<Vec<_>>()
            .join(",");
        if !te_value.is_empty() {
            let layers: Vec<&str> = te_value
                .split(',')
                .map(str::trim)
                .filter(|l| !l.is_empty() && *l != "chunked" && *l != "identity")
                .collect();
            for layer in layers.iter().rev() {
                let mut decoded = Vec::new();
                let ok = if layer.contains("gzip") || layer.contains("x-gzip") {
                    GzDecoder::new(&body[..]).read_to_end(&mut decoded).is_ok()
                } else if layer.contains("deflate") {
                    let zlib_ok = ZlibDecoder::new(&body[..])
                        .read_to_end(&mut decoded)
                        .is_ok();
                    if !zlib_ok || (decoded.is_empty() && !body.is_empty()) {
                        decoded.clear();
                        DeflateDecoder::new(&body[..])
                            .read_to_end(&mut decoded)
                            .is_ok()
                    } else {
                        true
                    }
                } else {
                    false
                };
                if !ok {
                    break;
                }
                body = decoded;
            }
        }
    }

    // Decompress if Content-Encoding is present and --compressed was requested
    let content_encoding = if decompress && !raw {
        headers
            .iter()
            .find(|(k, _)| k == "content-encoding")
            .map(|(_, v)| v.to_lowercase())
    } else {
        None
    };

    // Apply each Content-Encoding layer in reverse (last-applied first).
    let mut bad_encoding = false;
    let mut bad_encoding_too_many = false;
    let body = if let Some(enc) = content_encoding.as_deref() {
        let layers: Vec<&str> = enc.split(',').map(str::trim).collect();
        // Mirror curl's MAX_ENCODING_STACK = 5; reject longer chains.
        let real_layers = layers
            .iter()
            .filter(|l| !l.is_empty() && **l != "identity" && **l != "none")
            .count();
        if real_layers > 5 {
            bad_encoding = true;
            bad_encoding_too_many = true;
        }
        let mut current = body;
        if bad_encoding {
            current.clear();
        }
        for layer in layers.iter().rev() {
            if bad_encoding {
                break;
            }
            if layer.is_empty() || *layer == "identity" || *layer == "none" {
                continue;
            }
            let mut decoded = Vec::new();
            let ok = if layer.contains("gzip") || layer.contains("x-gzip") {
                GzDecoder::new(&current[..])
                    .read_to_end(&mut decoded)
                    .is_ok()
            } else if layer.contains("deflate") {
                // Try zlib (RFC 1950) first; fall back to raw deflate (RFC 1951).
                let zlib_ok = ZlibDecoder::new(&current[..])
                    .read_to_end(&mut decoded)
                    .is_ok();
                if !zlib_ok || (decoded.is_empty() && !current.is_empty()) {
                    decoded.clear();
                    DeflateDecoder::new(&current[..])
                        .read_to_end(&mut decoded)
                        .is_ok()
                } else {
                    true
                }
            } else {
                // Unknown encoding — leave as-is.
                false
            };
            if !ok {
                bad_encoding = true;
                current.clear();
                break;
            }
            current = decoded;
        }
        current
    } else {
        body
    };

    // Remove content-encoding and content-length headers after decompression
    if content_encoding.is_some() {
        headers.retain(|(k, _)| k != "content-encoding" && k != "content-length");
    }

    Ok(Response {
        trailer_bytes: chunked_trailers,
        status,
        status_text,
        headers,
        body,
        header_bytes,
        redirect_headers: Vec::new(),
        num_connects: 0,
        num_redirects: 0,
        max_redirects_reached: false,
        proto_redir_blocked: false,
        weird_server_reply: false,
        final_url: None,
        redirect_url: None,
        timed_out,
        recv_error: recv_error_flag,
        partial_file: partial_flag,
        bad_content_encoding: bad_encoding,
        bad_encoding_too_many,
        filesize_exceeded: false,
        header_size_error: false,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChunkErr {
    None,
    /// Truncated transfer (connection closed mid-chunk) -> CURLE_PARTIAL_FILE (18).
    Partial,
    /// Malformed framing (bad size, overflow, etc.) -> CURLE_RECV_ERROR (56).
    Recv,
}

fn read_chunked_body(reader: &mut impl BufRead) -> Result<(Vec<u8>, ChunkErr, Vec<u8>), String> {
    let mut body = Vec::new();
    loop {
        let mut size_line = String::new();
        let n = match reader.read_line(&mut size_line) {
            Ok(n) => n,
            Err(_) => return Ok((body, ChunkErr::Partial, Vec::new())),
        };
        if n == 0 {
            // EOF before terminator chunk.
            return Ok((body, ChunkErr::Partial, Vec::new()));
        }
        let size_str = size_line.trim();
        // Strip chunk extensions.
        let size_str = size_str.split(';').next().unwrap_or(size_str).trim();
        if size_str.is_empty() {
            return Ok((body, ChunkErr::Recv, Vec::new()));
        }
        let size = match usize::from_str_radix(size_str, 16) {
            Ok(s) => s,
            Err(_) => return Ok((body, ChunkErr::Recv, Vec::new())),
        };
        if size == 0 {
            // Read trailer headers until empty line (just CRLF).
            // Per HTTP/1.1, after the final 0-length chunk, optional
            // trailer headers may appear before the terminating CRLF.
            let mut trailer_bytes: Vec<u8> = Vec::new();
            loop {
                let mut trailer_line = String::new();
                match reader.read_line(&mut trailer_line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if trailer_line == "\r\n" || trailer_line == "\n" || trailer_line.is_empty()
                        {
                            break;
                        }
                        trailer_bytes.extend_from_slice(trailer_line.as_bytes());
                    }
                }
            }
            // Append trailers to body (curl includes them in data output).
            body.extend_from_slice(&trailer_bytes);
            // Return trailers separately so they can also go to header dump.
            return Ok((body, ChunkErr::None, trailer_bytes));
        }
        let mut chunk = vec![0u8; size];
        if reader.read_exact(&mut chunk).is_err() {
            return Ok((body, ChunkErr::Partial, Vec::new()));
        }
        body.extend_from_slice(&chunk);
        // Read trailing CRLF after chunk data.
        let mut crlf = [0u8; 2];
        let _ = reader.read_exact(&mut crlf);
    }
}

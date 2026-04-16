use std::io::{BufRead, BufReader, Read};

use crate::connection::Connection;

pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
    pub(crate) header_bytes: Vec<u8>,
    /// Collected header bytes from intermediate redirect responses.
    pub(crate) redirect_headers: Vec<u8>,
}

pub(crate) fn read_response(conn: &mut Connection) -> Result<Response, String> {
    let mut reader = BufReader::new(conn);

    // Read status line.
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .map_err(|e| format!("failed to read status line: {e}"))?;

    // Preserve original line endings in header_bytes (the server may send \n or \r\n)
    let mut header_bytes = Vec::new();
    header_bytes.extend_from_slice(status_line.as_bytes());

    let status_line = status_line.trim_end().to_string();

    let parts: Vec<&str> = status_line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(format!("malformed status line: {status_line}"));
    }
    let status: u16 = parts[1]
        .parse()
        .map_err(|_| format!("invalid status code: {}", parts[1]))?;
    let status_text = if parts.len() > 2 {
        parts[2].to_string()
    } else {
        String::new()
    };

    // Read headers.
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("failed to read header: {e}"))?;
        header_bytes.extend_from_slice(line.as_bytes());
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((key, val)) = trimmed.split_once(':') {
            headers.push((key.trim().to_lowercase(), val.trim().to_string()));
        }
    }

    // Read body based on Transfer-Encoding or Content-Length.
    let is_chunked = headers
        .iter()
        .any(|(k, v)| k == "transfer-encoding" && v.contains("chunked"));
    let content_length: Option<usize> = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok());

    let body = if is_chunked {
        read_chunked_body(&mut reader)?
    } else if let Some(len) = content_length {
        let mut buf = vec![0u8; len];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("failed to read body: {e}"))?;
        buf
    } else {
        // Read until EOF.
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
        buf
    };

    Ok(Response {
        status,
        status_text,
        headers,
        body,
        header_bytes,
        redirect_headers: Vec::new(),
    })
}

fn read_chunked_body(reader: &mut impl BufRead) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let mut size_line = String::new();
        reader
            .read_line(&mut size_line)
            .map_err(|e| format!("failed to read chunk size: {e}"))?;
        let size_str = size_line.trim();
        // Strip chunk extensions.
        let size_str = size_str.split(';').next().unwrap_or(size_str);
        let size = usize::from_str_radix(size_str, 16)
            .map_err(|_| format!("bad chunk size: {size_str}"))?;
        if size == 0 {
            // Read trailing CRLF.
            let mut trailer = String::new();
            let _ = reader.read_line(&mut trailer);
            break;
        }
        let mut chunk = vec![0u8; size];
        reader
            .read_exact(&mut chunk)
            .map_err(|e| format!("failed to read chunk: {e}"))?;
        body.extend_from_slice(&chunk);
        // Read trailing CRLF after chunk data.
        let mut crlf = [0u8; 2];
        let _ = reader.read_exact(&mut crlf);
    }
    Ok(body)
}

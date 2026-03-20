#![allow(dead_code)]
//! Minimal native D-Bus wire protocol client.
//!
//! Implements enough of the D-Bus protocol to make method calls on the
//! session and system buses without requiring external tools (gdbus/busctl).
//!
//! Supports:
//! - Unix socket connection with SASL EXTERNAL authentication
//! - Hello() to get a unique bus name
//! - Method calls with basic argument types (string, uint32, variant, array)
//! - Blocking reply reading

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

// D-Bus message types.
const METHOD_CALL: u8 = 1;
const METHOD_RETURN: u8 = 2;
const ERROR: u8 = 3;

// D-Bus header field codes.
const HEADER_PATH: u8 = 1;
const HEADER_INTERFACE: u8 = 2;
const HEADER_MEMBER: u8 = 3;
const HEADER_DESTINATION: u8 = 6;
const HEADER_SIGNATURE: u8 = 8;

// Protocol constants.
const LITTLE_ENDIAN_FLAG: u8 = b'l';
const PROTOCOL_VERSION: u8 = 1;
const NO_REPLY_EXPECTED: u32 = 0x1;

/// A D-Bus connection.
pub struct Connection {
    stream: UnixStream,
    serial: u32,
    unique_name: String,
}

/// A D-Bus method call result.
#[derive(Debug)]
pub enum CallResult {
    /// Successful return with body bytes.
    Return(Vec<u8>),
    /// Error with name and message.
    Error(String, String),
}

impl Connection {
    /// Connect to the session bus.
    pub fn session() -> Result<Self, String> {
        let addr = std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .map_err(|_| "DBUS_SESSION_BUS_ADDRESS not set")?;

        let path = addr
            .strip_prefix("unix:path=")
            .or_else(|| addr.split(',').find_map(|p| p.strip_prefix("path=")))
            .ok_or_else(|| format!("cannot parse bus address: {addr}"))?;

        Self::connect(path)
    }

    /// Connect to the system bus.
    pub fn system() -> Result<Self, String> {
        let path = if let Ok(addr) = std::env::var("DBUS_SYSTEM_BUS_ADDRESS") {
            addr.strip_prefix("unix:path=")
                .unwrap_or("/run/dbus/system_bus_socket")
                .to_string()
        } else {
            "/run/dbus/system_bus_socket".to_string()
        };

        Self::connect(&path)
    }

    /// Connect to a bus at the given Unix socket path.
    fn connect(path: &str) -> Result<Self, String> {
        let stream = UnixStream::connect(path).map_err(|e| format!("connect {path}: {e}"))?;

        let mut conn = Connection {
            stream,
            serial: 0,
            unique_name: String::new(),
        };

        // SASL EXTERNAL authentication.
        conn.authenticate()?;

        // Send Hello to get unique name.
        let result = conn.call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "Hello",
            &[],
            "",
        )?;

        if let CallResult::Return(body) = result {
            // Parse the unique name string from the body.
            if body.len() >= 4 {
                let len = u32::from_le_bytes(body[0..4].try_into().unwrap()) as usize;
                if body.len() >= 4 + len {
                    conn.unique_name = String::from_utf8_lossy(&body[4..4 + len]).to_string();
                }
            }
        }

        Ok(conn)
    }

    fn authenticate(&mut self) -> Result<(), String> {
        let uid = unsafe { libc::getuid() };
        let uid_hex: String = uid
            .to_string()
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect();

        // Send NUL byte to start.
        self.stream
            .write_all(b"\0")
            .map_err(|e| format!("write NUL: {e}"))?;

        // Send AUTH EXTERNAL.
        let auth = format!("AUTH EXTERNAL {uid_hex}\r\n");
        self.stream
            .write_all(auth.as_bytes())
            .map_err(|e| format!("write AUTH: {e}"))?;

        // Read response.
        let mut buf = [0u8; 256];
        let n = self
            .stream
            .read(&mut buf)
            .map_err(|e| format!("read AUTH response: {e}"))?;
        let response = String::from_utf8_lossy(&buf[..n]);

        if !response.starts_with("OK") {
            return Err(format!("SASL auth failed: {response}"));
        }

        // Begin.
        self.stream
            .write_all(b"BEGIN\r\n")
            .map_err(|e| format!("write BEGIN: {e}"))?;

        Ok(())
    }

    fn next_serial(&mut self) -> u32 {
        self.serial += 1;
        self.serial
    }

    /// Make a method call and wait for the reply.
    pub fn call(
        &mut self,
        destination: &str,
        path: &str,
        interface: &str,
        method: &str,
        args: &[&[u8]],
        signature: &str,
    ) -> Result<CallResult, String> {
        let serial = self.next_serial();

        // Build the message.
        let msg = build_method_call(
            serial,
            destination,
            path,
            interface,
            method,
            args,
            signature,
        );

        self.stream
            .write_all(&msg)
            .map_err(|e| format!("write message: {e}"))?;

        // Read reply.
        self.read_reply(serial)
    }

    fn read_reply(&mut self, _expected_serial: u32) -> Result<CallResult, String> {
        // Read the 16-byte fixed header.
        let mut header = [0u8; 16];
        self.stream
            .read_exact(&mut header)
            .map_err(|e| format!("read header: {e}"))?;

        let msg_type = header[1];
        let body_len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        let _reply_serial = u32::from_le_bytes(header[8..12].try_into().unwrap());

        // Read header fields array.
        let header_fields_len = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
        let mut header_fields = vec![0u8; header_fields_len];
        if header_fields_len > 0 {
            self.stream
                .read_exact(&mut header_fields)
                .map_err(|e| format!("read header fields: {e}"))?;
        }

        // Align to 8 bytes after header fields.
        let total_header = 16 + header_fields_len;
        let padding = (8 - (total_header % 8)) % 8;
        if padding > 0 {
            let mut pad = vec![0u8; padding];
            let _ = self.stream.read_exact(&mut pad);
        }

        // Read body.
        let mut body = vec![0u8; body_len];
        if body_len > 0 {
            self.stream
                .read_exact(&mut body)
                .map_err(|e| format!("read body: {e}"))?;
        }

        match msg_type {
            METHOD_RETURN => Ok(CallResult::Return(body)),
            ERROR => {
                let error_name = extract_string(&header_fields).unwrap_or_default();
                let error_msg = extract_string(&body).unwrap_or_default();
                Ok(CallResult::Error(error_name, error_msg))
            }
            _ => Err(format!("unexpected message type: {msg_type}")),
        }
    }
}

/// Build a D-Bus method call message.
fn build_method_call(
    serial: u32,
    destination: &str,
    path: &str,
    interface: &str,
    method: &str,
    args: &[&[u8]],
    signature: &str,
) -> Vec<u8> {
    // Build body from args.
    let mut body = Vec::new();
    for arg in args {
        body.extend_from_slice(arg);
    }

    // Build header fields array.
    let mut fields = Vec::new();
    append_header_field(&mut fields, HEADER_PATH, path);
    append_header_field(&mut fields, HEADER_INTERFACE, interface);
    append_header_field(&mut fields, HEADER_MEMBER, method);
    append_header_field(&mut fields, HEADER_DESTINATION, destination);
    if !signature.is_empty() {
        // Signature field uses 'g' type (signature), not 's'.
        fields.push(HEADER_SIGNATURE);
        fields.push(1); // variant signature length
        fields.push(b'g'); // type 'g'
        fields.push(0); // NUL
        fields.push(signature.len() as u8);
        fields.extend_from_slice(signature.as_bytes());
        fields.push(0);
        // Pad to 8.
        while fields.len() % 8 != 0 {
            fields.push(0);
        }
    }

    // Fixed header: endianness, type, flags, version, body_len, serial, fields_len.
    let mut msg = vec![
        LITTLE_ENDIAN_FLAG,
        METHOD_CALL,
        0, // flags
        PROTOCOL_VERSION,
    ];
    msg.extend_from_slice(&(body.len() as u32).to_le_bytes());
    msg.extend_from_slice(&serial.to_le_bytes());
    msg.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    msg.extend_from_slice(&fields);

    // Align body to 8 bytes.
    while !msg.len().is_multiple_of(8) {
        msg.push(0);
    }

    msg.extend_from_slice(&body);
    msg
}

/// Append a string-typed header field.
fn append_header_field(fields: &mut Vec<u8>, code: u8, value: &str) {
    // Align to 8 bytes.
    while !fields.len().is_multiple_of(8) {
        fields.push(0);
    }
    fields.push(code);
    // Variant signature: 1 byte length + "s" + NUL.
    fields.push(1);
    fields.push(b's');
    fields.push(0);
    // String value: 4-byte length + string + NUL.
    fields.extend_from_slice(&(value.len() as u32).to_le_bytes());
    fields.extend_from_slice(value.as_bytes());
    fields.push(0);
}

/// Extract a string from D-Bus marshalled data.
fn extract_string(data: &[u8]) -> Option<String> {
    if data.len() < 4 {
        return None;
    }
    let len = u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;
    if data.len() < 4 + len {
        return None;
    }
    Some(String::from_utf8_lossy(&data[4..4 + len]).to_string())
}

/// Serialize a string as D-Bus marshalled bytes.
pub fn marshal_string(s: &str) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&(s.len() as u32).to_le_bytes());
    data.extend_from_slice(s.as_bytes());
    data.push(0);
    data
}

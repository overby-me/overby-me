//! MsgPack-RPC protocol implementation.
//!
//! ## Wire format
//!
//! All messages are length-prefixed:
//!
//! ```text
//! ┌──────────────────┬──────────────────────────┐
//! │ 4 bytes BE u32   │  MessagePack payload      │
//! │ (payload length) │  (Request | Response | …) │
//! └──────────────────┴──────────────────────────┘
//! ```
//!
//! ## Message types
//!
//! - **Request** (client → agent): `{ version: "2.0", id: N, method: "...", params: {...} }`
//! - **Response** (agent → client): `{ version: "2.0", id: N, result: ... }` or `{ ..., error: { code, message } }`
//! - **Notification** (agent → client): `{ version: "2.0", method: "...", params: {...} }` (no `id`)
//!
//! Binary data (file contents) uses MsgPack's native `bin` type — no base64.

use rmpv::Value;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// RPC-layer errors (framing, encoding, protocol violations).
#[derive(Debug, Error)]
pub enum RpcError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("msgpack encode error: {0}")]
    Encode(#[from] rmp_serde::encode::Error),

    #[error("msgpack decode error: {0}")]
    Decode(#[from] rmp_serde::decode::Error),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("connection closed")]
    ConnectionClosed,
}

pub type RpcResult<T> = Result<T, RpcError>;

// ---------------------------------------------------------------------------
// Standard error codes (loosely based on JSON-RPC 2.0)
// ---------------------------------------------------------------------------

/// Well-known error codes sent in [`RpcErrorData`].
pub mod error_code {
    /// The method name is not recognised.
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// The parameters are invalid or missing.
    pub const INVALID_PARAMS: i32 = -32602;
    /// An internal / unexpected error occurred.
    pub const INTERNAL_ERROR: i32 = -32603;

    // Application-defined codes (≥ -32000):
    /// The target file/directory was not found.
    pub const NOT_FOUND: i32 = -32000;
    /// Permission denied on the remote filesystem.
    pub const PERMISSION_DENIED: i32 = -32001;
    /// Generic I/O error on the remote filesystem.
    pub const IO_ERROR: i32 = -32002;
}

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

/// A request message sent from client to agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// Protocol version — always `"2.0"`.
    pub version: String,
    /// Unique request identifier (monotonically increasing).
    pub id: u64,
    /// The RPC method to invoke (e.g. `"file.read"`, `"dir.list"`).
    pub method: String,
    /// Method parameters as a MsgPack map.
    pub params: Value,
}

/// A successful response from agent to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    /// Protocol version — always `"2.0"`.
    pub version: String,
    /// Matches the `id` of the originating [`Request`].
    pub id: u64,
    /// The result payload (structure depends on the method).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Present only when the operation failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcErrorData>,
}

/// An unsolicited notification from agent to client (no `id`, no response
/// expected).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    /// Protocol version — always `"2.0"`.
    pub version: String,
    /// Notification type (e.g. `"fs.changed"`).
    pub method: String,
    /// Associated payload.
    pub params: Value,
}

/// Error payload inside a [`Response`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcErrorData {
    /// Machine-readable error code (see [`error_code`]).
    pub code: i32,
    /// Human-readable error message.
    pub message: String,
}

// ---------------------------------------------------------------------------
// Constructors
// ---------------------------------------------------------------------------

impl Request {
    /// Create a new request.
    #[allow(dead_code)] // Used in tests and by future RPC client code
    pub fn new(id: u64, method: impl Into<String>, params: Value) -> Self {
        Self {
            version: "2.0".into(),
            id,
            method: method.into(),
            params,
        }
    }
}

impl Response {
    /// Create a successful response.
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            version: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Create an error response.
    pub fn err(id: u64, code: i32, message: impl Into<String>) -> Self {
        Self {
            version: "2.0".into(),
            id,
            result: None,
            error: Some(RpcErrorData {
                code,
                message: message.into(),
            }),
        }
    }
}

impl Notification {
    /// Create a new notification.
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            version: "2.0".into(),
            method: method.into(),
            params,
        }
    }
}

// ---------------------------------------------------------------------------
// Incoming message (tagged union for the reader)
// ---------------------------------------------------------------------------

/// A message received on the wire — could be a request or a notification.
///
/// The agent only ever *receives* requests (and the client only ever
/// *receives* responses and notifications), but having a single enum
/// simplifies the read path.
#[derive(Debug, Clone)]
pub enum Incoming {
    Request(Request),
}

// ---------------------------------------------------------------------------
// Framing — async read / write
// ---------------------------------------------------------------------------

/// Maximum payload size (64 MiB) to prevent malicious / buggy senders from
/// exhausting memory.
const MAX_PAYLOAD_SIZE: u32 = 64 * 1024 * 1024;

/// Read a single length-prefixed MsgPack message from `reader`.
///
/// Returns `Err(RpcError::ConnectionClosed)` on clean EOF (zero-length read
/// on the length prefix).
pub async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> RpcResult<Incoming> {
    // 1. Read 4-byte big-endian length prefix.
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(RpcError::ConnectionClosed);
        }
        Err(e) => return Err(RpcError::Io(e)),
    }
    let len = u32::from_be_bytes(len_buf);

    if len == 0 {
        return Err(RpcError::Protocol("zero-length payload".into()));
    }
    if len > MAX_PAYLOAD_SIZE {
        return Err(RpcError::Protocol(format!(
            "payload too large: {len} bytes (max {MAX_PAYLOAD_SIZE})"
        )));
    }

    // 2. Read the payload.
    let mut buf = vec![0u8; len as usize];
    reader.read_exact(&mut buf).await?;

    // 3. Deserialize as a generic MsgPack map to inspect the shape.
    let value: Value = rmp_serde::from_slice(&buf)?;

    // Determine message type by looking for the `id` and `method` fields.
    let map = value
        .as_map()
        .ok_or_else(|| RpcError::Protocol("expected a MsgPack map".into()))?;

    let has_id = map
        .iter()
        .any(|(k, _)| k.as_str().is_some_and(|s| s == "id"));
    let has_method = map
        .iter()
        .any(|(k, _)| k.as_str().is_some_and(|s| s == "method"));

    if has_id && has_method {
        // It's a Request.
        let req: Request = rmp_serde::from_slice(&buf)?;
        Ok(Incoming::Request(req))
    } else {
        Err(RpcError::Protocol(
            "message is neither a valid request nor notification".into(),
        ))
    }
}

/// Write a [`Response`] as a length-prefixed MsgPack message.
pub async fn write_response<W: AsyncWrite + Unpin>(
    writer: &mut W,
    response: &Response,
) -> RpcResult<()> {
    write_frame(writer, response).await
}

/// Write a [`Notification`] as a length-prefixed MsgPack message.
pub async fn write_notification<W: AsyncWrite + Unpin>(
    writer: &mut W,
    notification: &Notification,
) -> RpcResult<()> {
    write_frame(writer, notification).await
}

/// Serialize `msg` with MsgPack, prepend a 4-byte BE length, and write both.
async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    msg: &T,
) -> RpcResult<()> {
    let payload = rmp_serde::to_vec_named(msg)?;
    let len = payload.len() as u32;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rmpv::Value;

    /// Round-trip a Request through write → read.
    #[tokio::test]
    async fn round_trip_request() {
        let req = Request::new(
            1,
            "file.stat",
            Value::Map(vec![(
                Value::String("path".into()),
                Value::String("/etc/hosts".into()),
            )]),
        );

        // Serialize to a buffer.
        let mut buf = Vec::new();
        write_frame(&mut buf, &req).await.unwrap();

        // Deserialize back.
        let mut cursor = std::io::Cursor::new(buf);
        let incoming = read_message(&mut cursor).await.unwrap();

        match incoming {
            Incoming::Request(r) => {
                assert_eq!(r.id, 1);
                assert_eq!(r.method, "file.stat");
                assert_eq!(r.version, "2.0");
                let map = r.params.as_map().unwrap();
                assert_eq!(map.len(), 1);
            }
        }
    }

    /// Round-trip a successful Response.
    #[tokio::test]
    async fn round_trip_ok_response() {
        let resp = Response::ok(42, Value::String("hello".into()));

        let mut buf = Vec::new();
        write_response(&mut buf, &resp).await.unwrap();

        // Manually decode to verify structure.
        let payload = &buf[4..];
        let value: Value = rmp_serde::from_slice(payload).unwrap();
        let map = value.as_map().unwrap();

        let id = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("id"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();
        assert_eq!(id, 42);

        let result = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("result"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert_eq!(result, "hello");

        // `error` key should not be present.
        let has_error = map.iter().any(|(k, _)| k.as_str() == Some("error"));
        assert!(!has_error);
    }

    /// Round-trip an error Response.
    #[tokio::test]
    async fn round_trip_err_response() {
        let resp = Response::err(7, error_code::NOT_FOUND, "no such file");

        let mut buf = Vec::new();
        write_response(&mut buf, &resp).await.unwrap();

        let payload = &buf[4..];
        let value: Value = rmp_serde::from_slice(payload).unwrap();
        let map = value.as_map().unwrap();

        let err = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("error"))
            .unwrap()
            .1
            .as_map()
            .unwrap();

        let code = err
            .iter()
            .find(|(k, _)| k.as_str() == Some("code"))
            .unwrap()
            .1
            .as_i64()
            .unwrap();
        assert_eq!(code, error_code::NOT_FOUND as i64);
    }

    /// EOF on the length prefix returns ConnectionClosed.
    #[tokio::test]
    async fn eof_returns_connection_closed() {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        let result = read_message(&mut cursor).await;
        assert!(matches!(result, Err(RpcError::ConnectionClosed)));
    }

    /// Zero-length payload is rejected.
    #[tokio::test]
    async fn zero_length_rejected() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&0u32.to_be_bytes());
        let mut cursor = std::io::Cursor::new(buf);
        let result = read_message(&mut cursor).await;
        assert!(matches!(result, Err(RpcError::Protocol(_))));
    }

    /// Oversized payload is rejected.
    #[tokio::test]
    async fn oversized_payload_rejected() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(MAX_PAYLOAD_SIZE + 1).to_be_bytes());
        let mut cursor = std::io::Cursor::new(buf);
        let result = read_message(&mut cursor).await;
        assert!(matches!(result, Err(RpcError::Protocol(_))));
    }

    /// Notification can be serialized and the framing is correct.
    #[tokio::test]
    async fn notification_framing() {
        let notif = Notification::new(
            "fs.changed",
            Value::Map(vec![(
                Value::String("paths".into()),
                Value::Array(vec![Value::String("/tmp/foo".into())]),
            )]),
        );

        let mut buf = Vec::new();
        write_notification(&mut buf, &notif).await.unwrap();

        // Check that the length prefix is correct.
        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        assert_eq!(len, buf.len() - 4);

        // Verify we can decode the payload.
        let payload = &buf[4..];
        let value: Value = rmp_serde::from_slice(payload).unwrap();
        let map = value.as_map().unwrap();

        let method = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("method"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert_eq!(method, "fs.changed");
    }
}

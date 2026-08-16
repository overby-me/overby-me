//! MsgPack-RPC client for communicating with the `tramp-agent` binary.
//!
//! This module implements the client side of the agent's length-prefixed
//! MsgPack-RPC protocol.  It is designed to work over any async
//! reader/writer pair (typically piped through an SSH session's
//! stdin/stdout).
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

use rmpv::Value;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::errors::{TrampError, TrampResult};

// ---------------------------------------------------------------------------
// Wire types (must match the agent's rpc module)
// ---------------------------------------------------------------------------

/// A request message sent from client to agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub version: String,
    pub id: u64,
    pub method: String,
    pub params: Value,
}

/// A response from agent to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub version: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcErrorData>,
}

/// Error payload inside a [`Response`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcErrorData {
    pub code: i32,
    pub message: String,
}

/// An unsolicited notification from agent to client (no `id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub version: String,
    pub method: String,
    pub params: Value,
}

/// Well-known error codes (matching the agent's `error_code` module).
pub mod error_code {
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
    pub const NOT_FOUND: i32 = -32000;
    pub const PERMISSION_DENIED: i32 = -32001;
    pub const IO_ERROR: i32 = -32002;
}

// ---------------------------------------------------------------------------
// Incoming message (tagged union for the reader)
// ---------------------------------------------------------------------------

/// A message received from the agent — either a response or a notification.
#[derive(Debug, Clone)]
pub enum Incoming {
    Response(Response),
    Notification(Notification),
}

// ---------------------------------------------------------------------------
// Framing — maximum payload size
// ---------------------------------------------------------------------------

/// Maximum payload size (64 MiB) to prevent runaway allocations.
const MAX_PAYLOAD_SIZE: u32 = 64 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Low-level framing helpers
// ---------------------------------------------------------------------------

/// Read a single length-prefixed MsgPack message from `reader`.
async fn read_incoming<R: AsyncRead + Unpin>(reader: &mut R) -> TrampResult<Incoming> {
    // 1. Read 4-byte big-endian length prefix.
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(TrampError::Internal(
                "RPC connection closed (EOF)".to_string(),
            ));
        }
        Err(e) => {
            return Err(TrampError::Internal(format!("RPC read error: {e}")));
        }
    }
    let len = u32::from_be_bytes(len_buf);

    if len == 0 {
        return Err(TrampError::Internal(
            "RPC protocol error: zero-length payload".to_string(),
        ));
    }
    if len > MAX_PAYLOAD_SIZE {
        return Err(TrampError::Internal(format!(
            "RPC payload too large: {len} bytes (max {MAX_PAYLOAD_SIZE})"
        )));
    }

    // 2. Read the payload.
    let mut buf = vec![0u8; len as usize];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|e| TrampError::Internal(format!("RPC read error (payload): {e}")))?;

    // 3. Deserialize as a generic MsgPack map to determine the message type.
    let value: Value = rmp_serde::from_slice(&buf)
        .map_err(|e| TrampError::Internal(format!("RPC decode error: {e}")))?;

    let map = value.as_map().ok_or_else(|| {
        TrampError::Internal("RPC protocol error: expected a MsgPack map".to_string())
    })?;

    let has_id = map
        .iter()
        .any(|(k, _)| k.as_str().is_some_and(|s| s == "id"));
    let has_method = map
        .iter()
        .any(|(k, _)| k.as_str().is_some_and(|s| s == "method"));

    if has_id && !has_method {
        // It's a Response.
        let resp: Response = rmp_serde::from_slice(&buf)
            .map_err(|e| TrampError::Internal(format!("RPC response decode error: {e}")))?;
        Ok(Incoming::Response(resp))
    } else if has_method && !has_id {
        // It's a Notification.
        let notif: Notification = rmp_serde::from_slice(&buf)
            .map_err(|e| TrampError::Internal(format!("RPC notification decode error: {e}")))?;
        Ok(Incoming::Notification(notif))
    } else if has_id && has_method {
        // Could be a request (unexpected from agent) — treat as error.
        Err(TrampError::Internal(
            "RPC protocol error: received a request from the agent (unexpected)".to_string(),
        ))
    } else {
        Err(TrampError::Internal(
            "RPC protocol error: message has neither id nor method".to_string(),
        ))
    }
}

/// Write a length-prefixed MsgPack message.
async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    msg: &T,
) -> TrampResult<()> {
    let payload = rmp_serde::to_vec_named(msg)
        .map_err(|e| TrampError::Internal(format!("RPC encode error: {e}")))?;
    let len = payload.len() as u32;
    writer
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|e| TrampError::Internal(format!("RPC write error (length): {e}")))?;
    writer
        .write_all(&payload)
        .await
        .map_err(|e| TrampError::Internal(format!("RPC write error (payload): {e}")))?;
    writer
        .flush()
        .await
        .map_err(|e| TrampError::Internal(format!("RPC flush error: {e}")))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// RPC Client
// ---------------------------------------------------------------------------

/// An RPC client that communicates with a running `tramp-agent` process
/// via length-prefixed MsgPack messages over an async reader/writer pair.
///
/// The client is designed to be used behind a `Mutex` — it processes
/// requests sequentially, sending one request and reading the response
/// before the next request can be sent.
///
/// Notifications received while waiting for a response are buffered and
/// can be drained separately (e.g. for `fs.changed` events).
pub struct RpcClient<R, W> {
    reader: Mutex<R>,
    writer: Mutex<W>,
    next_id: AtomicU64,
    /// Buffered notifications received while waiting for responses.
    notifications: Mutex<Vec<Notification>>,
}

impl<R, W> RpcClient<R, W>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    /// Create a new RPC client wrapping the given reader/writer pair.
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader: Mutex::new(reader),
            writer: Mutex::new(writer),
            next_id: AtomicU64::new(1),
            notifications: Mutex::new(Vec::new()),
        }
    }

    /// Send a request and wait for its response.
    ///
    /// Any notifications received while waiting are buffered internally.
    pub async fn call(&self, method: &str, params: Value) -> TrampResult<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let request = Request {
            version: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        };

        // Send the request.
        {
            let mut writer = self.writer.lock().await;
            write_frame(&mut *writer, &request).await?;
        }

        // Read messages until we get the matching response.
        let mut reader = self.reader.lock().await;
        loop {
            let incoming = read_incoming(&mut *reader).await?;
            match incoming {
                Incoming::Response(resp) => {
                    if resp.id != id {
                        // Response for a different request — this shouldn't
                        // happen in our sequential model, but handle it
                        // gracefully by logging and continuing.
                        continue;
                    }

                    // Check for RPC-level error.
                    if let Some(err) = resp.error {
                        return Err(rpc_error_to_tramp(err));
                    }

                    return Ok(resp.result.unwrap_or(Value::Nil));
                }
                Incoming::Notification(notif) => {
                    // Buffer the notification for later processing.
                    self.notifications.lock().await.push(notif);
                }
            }
        }
    }

    /// Send a ping request and verify the agent is alive.
    pub async fn ping(&self) -> TrampResult<()> {
        let result = self.call("ping", Value::Map(vec![])).await?;
        // Verify we got an "ok" status back.
        let status = result
            .as_map()
            .and_then(|m| {
                m.iter()
                    .find(|(k, _)| k.as_str() == Some("status"))
                    .and_then(|(_, v)| v.as_str())
            })
            .unwrap_or("");
        if status == "ok" {
            Ok(())
        } else {
            Err(TrampError::Internal(format!(
                "agent ping returned unexpected status: {status}"
            )))
        }
    }

    /// Drain any buffered notifications.
    pub async fn drain_notifications(&self) -> Vec<Notification> {
        let mut notifs = self.notifications.lock().await;
        std::mem::take(&mut *notifs)
    }
}

// ---------------------------------------------------------------------------
// Helper constructors for MsgPack params
// ---------------------------------------------------------------------------

/// Build a MsgPack map from key-value pairs.
///
/// Usage: `params![("path", val_str("/etc/hosts")), ("recursive", val_bool(true))]`
pub fn make_params(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(k, v)| (Value::String(k.into()), v))
            .collect(),
    )
}

/// Create a MsgPack string value.
pub fn val_str(s: &str) -> Value {
    Value::String(s.into())
}

/// Create a MsgPack binary value.
pub fn val_bin(data: &[u8]) -> Value {
    Value::Binary(data.to_vec())
}

/// Create a MsgPack boolean value.
pub fn val_bool(b: bool) -> Value {
    Value::Boolean(b)
}

/// Create a MsgPack unsigned integer value.
pub fn val_u64(n: u64) -> Value {
    Value::Integer(n.into())
}

/// Create a MsgPack array of strings.
pub fn val_str_array(items: &[&str]) -> Value {
    Value::Array(items.iter().map(|s| Value::String((*s).into())).collect())
}

// ---------------------------------------------------------------------------
// Response value extraction helpers
// ---------------------------------------------------------------------------

/// Extract a string field from a MsgPack map value.
pub fn get_str<'a>(map: &'a [(Value, Value)], key: &str) -> Option<&'a str> {
    map.iter()
        .find(|(k, _)| k.as_str() == Some(key))
        .and_then(|(_, v)| v.as_str())
}

/// Extract a u64 field from a MsgPack map value.
pub fn get_u64(map: &[(Value, Value)], key: &str) -> Option<u64> {
    map.iter()
        .find(|(k, _)| k.as_str() == Some(key))
        .and_then(|(_, v)| v.as_u64())
}

/// Extract a binary field from a MsgPack map value.
pub fn get_bin<'a>(map: &'a [(Value, Value)], key: &str) -> Option<&'a [u8]> {
    map.iter()
        .find(|(k, _)| k.as_str() == Some(key))
        .and_then(|(_, v)| v.as_slice())
}

/// Extract a boolean field from a MsgPack map value.
pub fn get_bool(map: &[(Value, Value)], key: &str) -> Option<bool> {
    map.iter()
        .find(|(k, _)| k.as_str() == Some(key))
        .and_then(|(_, v)| v.as_bool())
}

/// Extract an i64 field from a MsgPack map value.
pub fn get_i64(map: &[(Value, Value)], key: &str) -> Option<i64> {
    map.iter()
        .find(|(k, _)| k.as_str() == Some(key))
        .and_then(|(_, v)| v.as_i64())
}

/// Extract an array field from a MsgPack map value.
pub fn get_array<'a>(map: &'a [(Value, Value)], key: &str) -> Option<&'a [Value]> {
    map.iter()
        .find(|(k, _)| k.as_str() == Some(key))
        .and_then(|(_, v)| v.as_array())
        .map(|a| a.as_slice())
}

/// Extract a nested map field from a MsgPack map value.
pub fn get_map<'a>(map: &'a [(Value, Value)], key: &str) -> Option<&'a [(Value, Value)]> {
    map.iter()
        .find(|(k, _)| k.as_str() == Some(key))
        .and_then(|(_, v)| v.as_map())
        .map(|m| m.as_slice())
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

/// Convert an RPC error into the appropriate `TrampError`.
fn rpc_error_to_tramp(err: RpcErrorData) -> TrampError {
    match err.code {
        error_code::NOT_FOUND => TrampError::NotFound(err.message),
        error_code::PERMISSION_DENIED => TrampError::PermissionDenied(err.message),
        error_code::METHOD_NOT_FOUND => {
            TrampError::Internal(format!("agent does not support method: {}", err.message))
        }
        error_code::INVALID_PARAMS => {
            TrampError::Internal(format!("invalid RPC parameters: {}", err.message))
        }
        _ => TrampError::RemoteError(err.message),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: serialize a Response into a length-prefixed MsgPack frame.
    fn make_response_frame(resp: &Response) -> Vec<u8> {
        let payload = rmp_serde::to_vec_named(resp).unwrap();
        let len = payload.len() as u32;
        let mut buf = Vec::new();
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&payload);
        buf
    }

    #[tokio::test]
    async fn call_returns_result() {
        // Prepare a mock response that will be read from the "reader".
        let resp = Response {
            version: "2.0".to_string(),
            id: 1,
            result: Some(Value::Map(vec![(
                Value::String("status".into()),
                Value::String("ok".into()),
            )])),
            error: None,
        };
        let response_bytes = make_response_frame(&resp);

        // The "writer" captures the outgoing request.
        let writer = Vec::<u8>::new();

        let client = RpcClient::new(std::io::Cursor::new(response_bytes), writer);

        let result = client.call("ping", Value::Map(vec![])).await.unwrap();
        let map = result.as_map().unwrap();
        let status = get_str(map, "status").unwrap();
        assert_eq!(status, "ok");
    }

    #[tokio::test]
    async fn call_returns_rpc_error() {
        let resp = Response {
            version: "2.0".to_string(),
            id: 1,
            result: None,
            error: Some(RpcErrorData {
                code: error_code::NOT_FOUND,
                message: "no such file: /tmp/missing".to_string(),
            }),
        };
        let response_bytes = make_response_frame(&resp);

        let client = RpcClient::new(std::io::Cursor::new(response_bytes), Vec::<u8>::new());

        let err = client
            .call("file.stat", Value::Map(vec![]))
            .await
            .unwrap_err();
        assert!(matches!(err, TrampError::NotFound(_)));
    }

    #[tokio::test]
    async fn ping_success() {
        let resp = Response {
            version: "2.0".to_string(),
            id: 1,
            result: Some(Value::Map(vec![
                (Value::String("status".into()), Value::String("ok".into())),
                (
                    Value::String("version".into()),
                    Value::String("0.1.0".into()),
                ),
                (Value::String("pid".into()), Value::Integer(12345u64.into())),
            ])),
            error: None,
        };
        let response_bytes = make_response_frame(&resp);

        let client = RpcClient::new(std::io::Cursor::new(response_bytes), Vec::<u8>::new());

        client.ping().await.unwrap();
    }

    #[test]
    fn make_params_builds_map() {
        let p = make_params(vec![
            ("path", val_str("/etc/hosts")),
            ("recursive", val_bool(true)),
        ]);
        let map = p.as_map().unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(get_str(map, "path"), Some("/etc/hosts"));
        assert_eq!(get_bool(map, "recursive"), Some(true));
    }

    #[test]
    fn rpc_error_maps_correctly() {
        let err = rpc_error_to_tramp(RpcErrorData {
            code: error_code::NOT_FOUND,
            message: "gone".into(),
        });
        assert!(matches!(err, TrampError::NotFound(_)));

        let err = rpc_error_to_tramp(RpcErrorData {
            code: error_code::PERMISSION_DENIED,
            message: "nope".into(),
        });
        assert!(matches!(err, TrampError::PermissionDenied(_)));

        let err = rpc_error_to_tramp(RpcErrorData {
            code: error_code::IO_ERROR,
            message: "disk full".into(),
        });
        assert!(matches!(err, TrampError::RemoteError(_)));
    }

    #[test]
    fn extraction_helpers() {
        let map = vec![
            (Value::String("name".into()), Value::String("foo".into())),
            (Value::String("size".into()), Value::Integer(42u64.into())),
            (Value::String("data".into()), Value::Binary(vec![1, 2, 3])),
            (Value::String("ok".into()), Value::Boolean(true)),
        ];

        assert_eq!(get_str(&map, "name"), Some("foo"));
        assert_eq!(get_u64(&map, "size"), Some(42));
        assert_eq!(get_bin(&map, "data"), Some([1u8, 2, 3].as_slice()));
        assert_eq!(get_bool(&map, "ok"), Some(true));
        assert_eq!(get_str(&map, "missing"), None);
    }
}

//! Pipeline log subscription.
//!
//! `sh.tangled.ci.subscribePipelineLogs` is an atproto event stream: a
//! WebSocket whose every binary message is **two concatenated CBOR maps**, a
//! header naming the frame type and a body carrying it. The spindle closes
//! the socket once the logs are fully delivered, so a finished pipeline's
//! logs can be replayed by simply subscribing to it.

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;

/// Start or end of a workflow step.
#[derive(Debug, Clone)]
pub struct LogControl {
    pub kind: String,
    pub step: i64,
    pub time: String,
    pub status: String,
    pub content: String,
    pub workflow: String,
    pub command: Option<String>,
}

/// One line of workflow output.
#[derive(Debug, Clone)]
pub struct LogData {
    pub step: i64,
    pub time: String,
    pub stream: String,
    pub content: String,
    pub workflow: String,
}

#[derive(Debug, Clone)]
pub enum LogEvent {
    Control(LogControl),
    Data(LogData),
}

/// Stream a pipeline's logs, calling `on_event` for each. Returns when the
/// spindle closes the stream.
pub async fn subscribe_pipeline_logs<F>(url: &str, mut on_event: F) -> Result<()>
where
    F: FnMut(LogEvent) -> Result<()>,
{
    let (stream, _) = tokio_tungstenite::connect_async(url)
        .await
        .with_context(|| format!("connect pipeline log subscription at {url}"))?;
    let (_, mut read) = stream.split();

    while let Some(message) = read.next().await {
        let message = match message {
            Ok(m) => m,
            // A spindle ends a completed log by dropping the connection, not
            // by sending a close frame, so a reset is the normal ending here
            // rather than a failure.
            Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed)
            | Err(tokio_tungstenite::tungstenite::Error::AlreadyClosed)
            | Err(tokio_tungstenite::tungstenite::Error::Protocol(
                tokio_tungstenite::tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
            )) => break,
            Err(tokio_tungstenite::tungstenite::Error::Io(e))
                if e.kind() == std::io::ErrorKind::ConnectionReset =>
            {
                break
            }
            Err(e) => return Err(anyhow!("read pipeline log: {e}")),
        };
        let payload = match message {
            tokio_tungstenite::tungstenite::Message::Binary(b) => b,
            tokio_tungstenite::tungstenite::Message::Close(_) => break,
            _ => continue,
        };
        // A frame we cannot parse is skipped rather than fatal: an unknown
        // frame type must not truncate the log.
        if let Some(event) = decode_frame(&payload)? {
            on_event(event)?;
        }
    }
    Ok(())
}

/// Decode one frame: header map, then body map. Returns None for frames that
/// are not `#control` or `#data`.
fn decode_frame(payload: &[u8]) -> Result<Option<LogEvent>> {
    let mut cursor = std::io::Cursor::new(payload);
    let header: ciborium::Value = match ciborium::from_reader(&mut cursor) {
        Ok(v) => v,
        Err(e) => return Err(anyhow!("decode pipeline log frame header: {e}")),
    };
    let body: ciborium::Value = match ciborium::from_reader(&mut cursor) {
        Ok(v) => v,
        // Header with no body: nothing to report.
        Err(_) => return Ok(None),
    };

    match map_str(&header, "t").as_deref() {
        Some("#control") => Ok(Some(LogEvent::Control(LogControl {
            kind: map_str(&body, "kind").unwrap_or_default(),
            step: map_int(&body, "step").unwrap_or_default(),
            time: map_str(&body, "time").unwrap_or_default(),
            status: map_str(&body, "status").unwrap_or_default(),
            content: map_str(&body, "content").unwrap_or_default(),
            workflow: map_str(&body, "workflow").unwrap_or_default(),
            command: map_str(&body, "command"),
        }))),
        Some("#data") => Ok(Some(LogEvent::Data(LogData {
            step: map_int(&body, "step").unwrap_or_default(),
            time: map_str(&body, "time").unwrap_or_default(),
            stream: map_str(&body, "stream").unwrap_or_default(),
            content: map_str(&body, "content").unwrap_or_default(),
            workflow: map_str(&body, "workflow").unwrap_or_default(),
        }))),
        _ => Ok(None),
    }
}

fn map_get<'a>(value: &'a ciborium::Value, key: &str) -> Option<&'a ciborium::Value> {
    value
        .as_map()?
        .iter()
        .find(|(k, _)| k.as_text() == Some(key))
        .map(|(_, v)| v)
}

fn map_str(value: &ciborium::Value, key: &str) -> Option<String> {
    map_get(value, key)?.as_text().map(str::to_string)
}

fn map_int(value: &ciborium::Value, key: &str) -> Option<i64> {
    map_get(value, key)?.as_integer()?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a frame the way the spindle does: two CBOR maps back to back.
    fn frame(header: ciborium::Value, body: ciborium::Value) -> Vec<u8> {
        let mut out = Vec::new();
        ciborium::into_writer(&header, &mut out).unwrap();
        ciborium::into_writer(&body, &mut out).unwrap();
        out
    }

    fn text(s: &str) -> ciborium::Value {
        ciborium::Value::Text(s.to_string())
    }

    #[test]
    fn decodes_a_data_frame() {
        let payload = frame(
            ciborium::Value::Map(vec![(text("t"), text("#data"))]),
            ciborium::Value::Map(vec![
                (text("step"), ciborium::Value::Integer(2.into())),
                (text("stream"), text("stdout")),
                (text("content"), text("hello")),
                (text("workflow"), text("publish.yml")),
                (text("time"), text("2026-08-15T21:53:16Z")),
            ]),
        );
        match decode_frame(&payload).unwrap().unwrap() {
            LogEvent::Data(d) => {
                assert_eq!(d.step, 2);
                assert_eq!(d.stream, "stdout");
                assert_eq!(d.content, "hello");
                assert_eq!(d.workflow, "publish.yml");
            }
            other => panic!("expected data, got {other:?}"),
        }
    }

    #[test]
    fn decodes_a_control_frame() {
        let payload = frame(
            ciborium::Value::Map(vec![(text("t"), text("#control"))]),
            ciborium::Value::Map(vec![
                (text("kind"), text("step-end")),
                (text("status"), text("failed")),
                (text("workflow"), text("publish.yml")),
                (text("step"), ciborium::Value::Integer(1.into())),
            ]),
        );
        match decode_frame(&payload).unwrap().unwrap() {
            LogEvent::Control(c) => {
                assert_eq!(c.kind, "step-end");
                assert_eq!(c.status, "failed");
                assert_eq!(c.step, 1);
            }
            other => panic!("expected control, got {other:?}"),
        }
    }

    #[test]
    fn skips_an_unknown_frame_type() {
        let payload = frame(
            ciborium::Value::Map(vec![(text("t"), text("#somethingelse"))]),
            ciborium::Value::Map(vec![(text("content"), text("x"))]),
        );
        assert!(decode_frame(&payload).unwrap().is_none());
    }
}

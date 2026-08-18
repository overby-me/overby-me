//! Log line types for workflow execution output.
//!
//! Matches the upstream Go `LogLine`, `LogKind`, and `StepStatus` types.
//! Log lines are written as newline-delimited JSON to per-workflow log files
//! and streamed over WebSocket to clients.
//!
//! There are two kinds of log lines:
//! - **Data** lines contain stdout/stderr output from a step.
//! - **Control** lines indicate step lifecycle events (start/end).

use serde::{Deserialize, Serialize};

use crate::step::{Step, StepKind};

/// The kind of a log line: data (step output) or control (step lifecycle).
///
/// Serialized as lowercase strings to match the upstream Go JSON format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogKind {
    /// Step log data (stdout/stderr output).
    Data,
    /// Step lifecycle control message (start/end).
    Control,
}

impl std::fmt::Display for LogKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogKind::Data => f.write_str("data"),
            LogKind::Control => f.write_str("control"),
        }
    }
}

/// Indicates the lifecycle status of a step in a control log line.
///
/// Serialized as lowercase strings to match the upstream Go JSON format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    /// The step has started executing.
    Start,
    /// The step has finished executing.
    End,
}

impl std::fmt::Display for StepStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepStatus::Start => f.write_str("start"),
            StepStatus::End => f.write_str("end"),
        }
    }
}

/// A single log line emitted during workflow execution.
///
/// Matches the upstream Go `LogLine` struct and its JSON serialization format.
/// Log lines are written as newline-delimited JSON (NDJSON) to log files and
/// streamed over WebSocket connections.
///
/// # Fields by kind
///
/// - **Data** lines have `stream` set (e.g. `"stdout"` or `"stderr"`).
/// - **Control** lines have `step_status`, `step_kind`, and `step_command` set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    /// Whether this is a data or control log line.
    pub kind: LogKind,

    /// The content of the log line.
    /// - For data lines: the actual stdout/stderr text.
    /// - For control lines: the step name.
    pub content: String,

    /// Timestamp when this log line was produced.
    pub time: chrono::DateTime<chrono::Utc>,

    /// Zero-based index of the step that produced this log line.
    pub step_id: usize,

    // -- Data-specific fields --
    /// The output stream name (e.g. `"stdout"`, `"stderr"`).
    /// Only set for [`LogKind::Data`] lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,

    // -- Control-specific fields --
    /// The lifecycle status of the step.
    /// Only set for [`LogKind::Control`] lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_status: Option<StepStatus>,

    /// The kind of step (system or user).
    /// Only set for [`LogKind::Control`] lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_kind: Option<StepKind>,

    /// The command string of the step.
    /// Only set for [`LogKind::Control`] lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_command: Option<String>,
}

impl LogLine {
    /// Create a new data log line from step output.
    ///
    /// Matches the upstream Go `NewDataLogLine` constructor.
    ///
    /// # Arguments
    /// * `step_id` — Zero-based index of the step.
    /// * `content` — The output text (a single line of stdout/stderr).
    /// * `stream` — The stream name (`"stdout"` or `"stderr"`).
    pub fn data(step_id: usize, content: impl Into<String>, stream: impl Into<String>) -> Self {
        Self {
            kind: LogKind::Data,
            content: content.into(),
            time: chrono::Utc::now(),
            step_id,
            stream: Some(stream.into()),
            step_status: None,
            step_kind: None,
            step_command: None,
        }
    }

    /// Create a new control log line for a step lifecycle event.
    ///
    /// Matches the upstream Go `NewControlLogLine` constructor.
    ///
    /// # Arguments
    /// * `step_id` — Zero-based index of the step.
    /// * `step` — The step whose lifecycle is being reported.
    /// * `status` — Whether the step is starting or ending.
    pub fn control(step_id: usize, step: &dyn Step, status: StepStatus) -> Self {
        Self {
            kind: LogKind::Control,
            content: step.name().to_owned(),
            time: chrono::Utc::now(),
            step_id,
            stream: None,
            step_status: Some(status),
            step_kind: Some(step.kind()),
            step_command: Some(step.command()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::step::{StepKind, UserStep};

    #[test]
    fn log_kind_display() {
        assert_eq!(LogKind::Data.to_string(), "data");
        assert_eq!(LogKind::Control.to_string(), "control");
    }

    #[test]
    fn step_status_display() {
        assert_eq!(StepStatus::Start.to_string(), "start");
        assert_eq!(StepStatus::End.to_string(), "end");
    }

    #[test]
    fn data_log_line_construction() {
        let line = LogLine::data(0, "hello world", "stdout");

        assert_eq!(line.kind, LogKind::Data);
        assert_eq!(line.content, "hello world");
        assert_eq!(line.step_id, 0);
        assert_eq!(line.stream.as_deref(), Some("stdout"));
        assert!(line.step_status.is_none());
        assert!(line.step_kind.is_none());
        assert!(line.step_command.is_none());
    }

    #[test]
    fn control_log_line_construction() {
        let step = UserStep::new("Run tests", "cargo test");
        let line = LogLine::control(1, &step, StepStatus::Start);

        assert_eq!(line.kind, LogKind::Control);
        assert_eq!(line.content, "Run tests");
        assert_eq!(line.step_id, 1);
        assert!(line.stream.is_none());
        assert_eq!(line.step_status, Some(StepStatus::Start));
        assert_eq!(line.step_kind, Some(StepKind::User));
        assert_eq!(line.step_command.as_deref(), Some("cargo test"));
    }

    #[test]
    fn data_log_line_serialization_roundtrip() {
        let line = LogLine::data(2, "test output", "stderr");
        let json = serde_json::to_string(&line).unwrap();
        let deserialized: LogLine = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.kind, LogKind::Data);
        assert_eq!(deserialized.content, "test output");
        assert_eq!(deserialized.step_id, 2);
        assert_eq!(deserialized.stream.as_deref(), Some("stderr"));
        assert!(deserialized.step_status.is_none());
        assert!(deserialized.step_kind.is_none());
        assert!(deserialized.step_command.is_none());
    }

    #[test]
    fn control_log_line_serialization_roundtrip() {
        let step = UserStep::new("Build", "cargo build");
        let line = LogLine::control(0, &step, StepStatus::End);
        let json = serde_json::to_string(&line).unwrap();
        let deserialized: LogLine = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.kind, LogKind::Control);
        assert_eq!(deserialized.content, "Build");
        assert_eq!(deserialized.step_id, 0);
        assert!(deserialized.stream.is_none());
        assert_eq!(deserialized.step_status, Some(StepStatus::End));
        assert_eq!(deserialized.step_kind, Some(StepKind::User));
        assert_eq!(deserialized.step_command.as_deref(), Some("cargo build"));
    }

    #[test]
    fn data_log_line_json_format() {
        let line = LogLine::data(0, "hello", "stdout");
        let json = serde_json::to_value(&line).unwrap();

        assert_eq!(json["kind"], "data");
        assert_eq!(json["content"], "hello");
        assert_eq!(json["step_id"], 0);
        assert_eq!(json["stream"], "stdout");
        // Control-specific fields should be absent (not null)
        assert!(json.get("step_status").is_none());
        assert!(json.get("step_kind").is_none());
        assert!(json.get("step_command").is_none());
    }

    #[test]
    fn control_log_line_json_format() {
        let step = UserStep::new("Lint", "cargo clippy");
        let line = LogLine::control(3, &step, StepStatus::Start);
        let json = serde_json::to_value(&line).unwrap();

        assert_eq!(json["kind"], "control");
        assert_eq!(json["content"], "Lint");
        assert_eq!(json["step_id"], 3);
        assert_eq!(json["step_status"], "start");
        assert_eq!(json["step_kind"], 1); // StepKind::User = 1
        assert_eq!(json["step_command"], "cargo clippy");
        // Data-specific fields should be absent
        assert!(json.get("stream").is_none());
    }

    #[test]
    fn log_kind_serialization() {
        assert_eq!(serde_json::to_string(&LogKind::Data).unwrap(), "\"data\"");
        assert_eq!(
            serde_json::to_string(&LogKind::Control).unwrap(),
            "\"control\""
        );
    }

    #[test]
    fn log_kind_deserialization() {
        assert_eq!(
            serde_json::from_str::<LogKind>("\"data\"").unwrap(),
            LogKind::Data
        );
        assert_eq!(
            serde_json::from_str::<LogKind>("\"control\"").unwrap(),
            LogKind::Control
        );
    }

    #[test]
    fn step_status_serialization() {
        assert_eq!(
            serde_json::to_string(&StepStatus::Start).unwrap(),
            "\"start\""
        );
        assert_eq!(serde_json::to_string(&StepStatus::End).unwrap(), "\"end\"");
    }

    #[test]
    fn step_status_deserialization() {
        assert_eq!(
            serde_json::from_str::<StepStatus>("\"start\"").unwrap(),
            StepStatus::Start
        );
        assert_eq!(
            serde_json::from_str::<StepStatus>("\"end\"").unwrap(),
            StepStatus::End
        );
    }

    #[test]
    fn log_line_has_timestamp() {
        let before = chrono::Utc::now();
        let line = LogLine::data(0, "test", "stdout");
        let after = chrono::Utc::now();

        assert!(line.time >= before);
        assert!(line.time <= after);
    }

    #[test]
    fn deserialization_with_missing_optional_fields() {
        // A minimal data log line JSON (no control fields)
        let json = r#"{
            "kind": "data",
            "content": "hello",
            "time": "2024-01-01T00:00:00Z",
            "step_id": 0,
            "stream": "stdout"
        }"#;
        let line: LogLine = serde_json::from_str(json).unwrap();
        assert_eq!(line.kind, LogKind::Data);
        assert_eq!(line.content, "hello");
        assert_eq!(line.stream.as_deref(), Some("stdout"));
        assert!(line.step_status.is_none());
        assert!(line.step_kind.is_none());
        assert!(line.step_command.is_none());
    }

    #[test]
    fn deserialization_control_with_missing_optional_fields() {
        // A minimal control log line JSON (no data fields)
        let json = r#"{
            "kind": "control",
            "content": "Build",
            "time": "2024-01-01T00:00:00Z",
            "step_id": 1,
            "step_status": "start",
            "step_kind": 1,
            "step_command": "cargo build"
        }"#;
        let line: LogLine = serde_json::from_str(json).unwrap();
        assert_eq!(line.kind, LogKind::Control);
        assert_eq!(line.content, "Build");
        assert!(line.stream.is_none());
        assert_eq!(line.step_status, Some(StepStatus::Start));
        assert_eq!(line.step_kind, Some(StepKind::User));
        assert_eq!(line.step_command.as_deref(), Some("cargo build"));
    }
}

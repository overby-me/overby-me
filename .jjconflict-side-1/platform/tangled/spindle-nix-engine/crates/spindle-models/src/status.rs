//! Workflow execution status types.
//!
//! Matches the upstream Go `StatusKind` type and its associated constants.
//! A workflow transitions through these states during execution:
//!
//! ```text
//! Pending → Running → Success
//!                   → Failed
//!                   → Timeout
//!                   → Cancelled
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};

/// The execution status of a workflow.
///
/// Serializes to/from lowercase strings to match the upstream Go JSON format
/// (e.g. `"pending"`, `"running"`, `"failed"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusKind {
    Pending,
    Running,
    Failed,
    Timeout,
    Cancelled,
    Success,
}

/// The set of statuses that indicate a workflow has not yet finished.
pub const START_STATES: [StatusKind; 2] = [StatusKind::Pending, StatusKind::Running];

/// The set of statuses that indicate a workflow has reached a terminal state.
pub const FINISH_STATES: [StatusKind; 4] = [
    StatusKind::Failed,
    StatusKind::Timeout,
    StatusKind::Cancelled,
    StatusKind::Success,
];

impl StatusKind {
    /// Returns `true` if this is a start state (`Pending` or `Running`).
    pub fn is_start(self) -> bool {
        START_STATES.contains(&self)
    }

    /// Returns `true` if this is a terminal/finish state
    /// (`Failed`, `Timeout`, `Cancelled`, or `Success`).
    pub fn is_finish(self) -> bool {
        FINISH_STATES.contains(&self)
    }

    /// Returns the string representation matching the upstream Go format.
    pub fn as_str(self) -> &'static str {
        match self {
            StatusKind::Pending => "pending",
            StatusKind::Running => "running",
            StatusKind::Failed => "failed",
            StatusKind::Timeout => "timeout",
            StatusKind::Cancelled => "cancelled",
            StatusKind::Success => "success",
        }
    }
}

impl fmt::Display for StatusKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_matches_go_format() {
        assert_eq!(StatusKind::Pending.to_string(), "pending");
        assert_eq!(StatusKind::Running.to_string(), "running");
        assert_eq!(StatusKind::Failed.to_string(), "failed");
        assert_eq!(StatusKind::Timeout.to_string(), "timeout");
        assert_eq!(StatusKind::Cancelled.to_string(), "cancelled");
        assert_eq!(StatusKind::Success.to_string(), "success");
    }

    #[test]
    fn is_start_states() {
        assert!(StatusKind::Pending.is_start());
        assert!(StatusKind::Running.is_start());
        assert!(!StatusKind::Failed.is_start());
        assert!(!StatusKind::Timeout.is_start());
        assert!(!StatusKind::Cancelled.is_start());
        assert!(!StatusKind::Success.is_start());
    }

    #[test]
    fn is_finish_states() {
        assert!(!StatusKind::Pending.is_finish());
        assert!(!StatusKind::Running.is_finish());
        assert!(StatusKind::Failed.is_finish());
        assert!(StatusKind::Timeout.is_finish());
        assert!(StatusKind::Cancelled.is_finish());
        assert!(StatusKind::Success.is_finish());
    }

    #[test]
    fn start_and_finish_are_disjoint() {
        for status in [
            StatusKind::Pending,
            StatusKind::Running,
            StatusKind::Failed,
            StatusKind::Timeout,
            StatusKind::Cancelled,
            StatusKind::Success,
        ] {
            // Every status must be exactly one of start or finish
            assert_ne!(
                status.is_start(),
                status.is_finish(),
                "{status} should be exactly one of start or finish"
            );
        }
    }

    #[test]
    fn serialization_roundtrip() {
        for status in [
            StatusKind::Pending,
            StatusKind::Running,
            StatusKind::Failed,
            StatusKind::Timeout,
            StatusKind::Cancelled,
            StatusKind::Success,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: StatusKind = serde_json::from_str(&json).unwrap();
            assert_eq!(status, deserialized);
        }
    }

    #[test]
    fn serializes_to_lowercase_string() {
        assert_eq!(
            serde_json::to_string(&StatusKind::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&StatusKind::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&StatusKind::Failed).unwrap(),
            "\"failed\""
        );
        assert_eq!(
            serde_json::to_string(&StatusKind::Timeout).unwrap(),
            "\"timeout\""
        );
        assert_eq!(
            serde_json::to_string(&StatusKind::Cancelled).unwrap(),
            "\"cancelled\""
        );
        assert_eq!(
            serde_json::to_string(&StatusKind::Success).unwrap(),
            "\"success\""
        );
    }

    #[test]
    fn deserializes_from_lowercase_string() {
        assert_eq!(
            serde_json::from_str::<StatusKind>("\"pending\"").unwrap(),
            StatusKind::Pending
        );
        assert_eq!(
            serde_json::from_str::<StatusKind>("\"running\"").unwrap(),
            StatusKind::Running
        );
        assert_eq!(
            serde_json::from_str::<StatusKind>("\"failed\"").unwrap(),
            StatusKind::Failed
        );
        assert_eq!(
            serde_json::from_str::<StatusKind>("\"timeout\"").unwrap(),
            StatusKind::Timeout
        );
        assert_eq!(
            serde_json::from_str::<StatusKind>("\"cancelled\"").unwrap(),
            StatusKind::Cancelled
        );
        assert_eq!(
            serde_json::from_str::<StatusKind>("\"success\"").unwrap(),
            StatusKind::Success
        );
    }

    #[test]
    fn deserialize_invalid_status_fails() {
        assert!(serde_json::from_str::<StatusKind>("\"invalid\"").is_err());
        assert!(serde_json::from_str::<StatusKind>("\"Pending\"").is_err());
        assert!(serde_json::from_str::<StatusKind>("\"RUNNING\"").is_err());
    }
}

//! Pipeline and pipeline identifier types.
//!
//! A pipeline is triggered when a repository is modified. It consists of several
//! workflows (from `.tangled/workflows/*.yml`) that execute in parallel.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::workflow::Workflow;

/// The AT Protocol NSID for pipeline records.
pub const PIPELINE_NSID: &str = "sh.tangled.pipeline";

/// Identifies a specific pipeline execution on a knot server.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PipelineId {
    /// The knot server hostname (e.g. `"example.com"`).
    pub knot: String,
    /// The record key identifying this pipeline on the knot.
    pub rkey: String,
}

impl PipelineId {
    /// Construct the AT Protocol URI for this pipeline.
    ///
    /// Format: `at://did:web:{knot}/{PIPELINE_NSID}/{rkey}`
    pub fn at_uri(&self) -> String {
        format!("at://did:web:{}/{}/{}", self.knot, PIPELINE_NSID, self.rkey)
    }
}

impl fmt::Display for PipelineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.knot, self.rkey)
    }
}

/// A pipeline groups all workflows for a single trigger event.
///
/// When a repo event (push, PR, manual) arrives, the spindle parses the
/// workflow manifests, validates the engine, and creates a `Pipeline` containing
/// one or more [`Workflow`]s to execute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pipeline {
    /// DID of the repository owner.
    pub repo_owner: String,
    /// Name of the repository.
    pub repo_name: String,
    /// Commit SHA to checkout (from the trigger event).
    ///
    /// For push triggers this is `new_sha`, for PR triggers this is `source_sha`.
    /// When `None`, the clone step fetches without a specific ref (fallback).
    #[serde(default)]
    pub commit_sha: Option<String>,
    /// Workflows grouped by this pipeline, to be executed in parallel.
    pub workflows: Vec<Workflow>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_id_at_uri() {
        let id = PipelineId {
            knot: "example.com".into(),
            rkey: "abc123".into(),
        };
        assert_eq!(
            id.at_uri(),
            "at://did:web:example.com/sh.tangled.pipeline/abc123"
        );
    }

    #[test]
    fn pipeline_id_display() {
        let id = PipelineId {
            knot: "example.com".into(),
            rkey: "abc123".into(),
        };
        assert_eq!(id.to_string(), "example.com:abc123");
    }

    #[test]
    fn pipeline_id_equality() {
        let a = PipelineId {
            knot: "example.com".into(),
            rkey: "abc123".into(),
        };
        let b = PipelineId {
            knot: "example.com".into(),
            rkey: "abc123".into(),
        };
        let c = PipelineId {
            knot: "other.com".into(),
            rkey: "abc123".into(),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn pipeline_id_serialization_roundtrip() {
        let id = PipelineId {
            knot: "example.com".into(),
            rkey: "abc123".into(),
        };
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: PipelineId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }
}

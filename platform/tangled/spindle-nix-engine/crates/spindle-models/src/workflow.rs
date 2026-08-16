//! Workflow identifier and workflow types.
//!
//! A workflow is a single `.tangled/workflows/*.yml` file within a pipeline.
//! Each workflow contains an ordered list of steps that execute serially.
//! Multiple workflows within a pipeline execute in parallel.

use std::collections::HashMap;
use std::fmt;

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use crate::pipeline::PipelineId;
use crate::step::Step;

/// Regex that replaces any character that is not `[a-zA-Z0-9_.-]` with `-`.
/// Matches the upstream Go normalization: `regexp.MustCompile(`[^a-zA-Z0-9_.-]`)`.
static NORMALIZE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-zA-Z0-9_.\-]").expect("invalid normalize regex"));

/// Normalize a string by replacing non-alphanumeric characters (except `_`, `.`, `-`)
/// with hyphens. Matches the upstream Go `normalize()` function.
fn normalize(name: &str) -> String {
    NORMALIZE_RE.replace_all(name, "-").into_owned()
}

/// Uniquely identifies a workflow within a pipeline.
///
/// The string representation is `{normalized_knot}-{rkey}-{normalized_name}`,
/// matching the upstream Go `WorkflowId.String()`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowId {
    /// The parent pipeline identifier.
    pub pipeline_id: PipelineId,
    /// The workflow name (from the YAML filename, e.g. `"test"`, `"lint"`).
    pub name: String,
}

impl WorkflowId {
    /// Create a new `WorkflowId`.
    pub fn new(pipeline_id: PipelineId, name: impl Into<String>) -> Self {
        Self {
            pipeline_id,
            name: name.into(),
        }
    }
}

impl fmt::Display for WorkflowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}-{}-{}",
            normalize(&self.pipeline_id.knot),
            self.pipeline_id.rkey,
            normalize(&self.name)
        )
    }
}

/// A workflow is a sequence of steps to execute, with associated metadata.
///
/// This is the internal representation used by the engine after parsing
/// a `Pipeline_Workflow` record from the AT Protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    /// The workflow name (from the YAML filename).
    pub name: String,
    /// Ordered list of steps to execute serially.
    pub steps: Vec<Box<dyn Step>>,
    /// Engine-specific data (e.g. parsed dependency map for the nix engine).
    /// Stored as an opaque JSON value so each engine can attach its own metadata.
    pub data: Option<serde_json::Value>,
    /// Environment variables to inject into every step of this workflow.
    /// These come from the pipeline trigger metadata (`PipelineEnvVars`)
    /// plus any workflow-level `env:` declarations.
    pub environment: HashMap<String, String>,
}

impl Workflow {
    /// Create a new workflow with the given name and no steps.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            steps: Vec::new(),
            data: None,
            environment: HashMap::new(),
        }
    }

    /// Add a step to this workflow.
    pub fn add_step(&mut self, step: impl Step + 'static) {
        self.steps.push(Box::new(step));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::PipelineId;
    use crate::step::{StepKind, UserStep};

    #[test]
    fn workflow_id_to_string_basic() {
        let wid = WorkflowId::new(
            PipelineId {
                knot: "example.com".into(),
                rkey: "abc123".into(),
            },
            "test",
        );
        assert_eq!(wid.to_string(), "example.com-abc123-test");
    }

    #[test]
    fn workflow_id_to_string_normalization() {
        // Characters not in [a-zA-Z0-9_.-] should be replaced with `-`
        let wid = WorkflowId::new(
            PipelineId {
                knot: "my knot:3000".into(),
                rkey: "rkey1".into(),
            },
            "build & test",
        );
        assert_eq!(wid.to_string(), "my-knot-3000-rkey1-build---test");
    }

    #[test]
    fn workflow_id_to_string_special_chars_preserved() {
        // Underscores, dots, and hyphens should be preserved
        let wid = WorkflowId::new(
            PipelineId {
                knot: "my_knot.example-com".into(),
                rkey: "rkey2".into(),
            },
            "build_test.yml",
        );
        assert_eq!(wid.to_string(), "my_knot.example-com-rkey2-build_test.yml");
    }

    #[test]
    fn workflow_id_equality() {
        let a = WorkflowId::new(
            PipelineId {
                knot: "example.com".into(),
                rkey: "abc".into(),
            },
            "test",
        );
        let b = WorkflowId::new(
            PipelineId {
                knot: "example.com".into(),
                rkey: "abc".into(),
            },
            "test",
        );
        let c = WorkflowId::new(
            PipelineId {
                knot: "example.com".into(),
                rkey: "abc".into(),
            },
            "lint",
        );
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn workflow_id_serialization_roundtrip() {
        let wid = WorkflowId::new(
            PipelineId {
                knot: "example.com".into(),
                rkey: "abc123".into(),
            },
            "test",
        );
        let json = serde_json::to_string(&wid).unwrap();
        let deserialized: WorkflowId = serde_json::from_str(&json).unwrap();
        assert_eq!(wid, deserialized);
    }

    #[test]
    fn workflow_new_has_no_steps() {
        let wf = Workflow::new("test");
        assert_eq!(wf.name, "test");
        assert!(wf.steps.is_empty());
        assert!(wf.data.is_none());
        assert!(wf.environment.is_empty());
    }

    #[test]
    fn workflow_add_step() {
        let mut wf = Workflow::new("build");
        wf.add_step(UserStep::new("Run tests", "cargo test"));
        wf.add_step(UserStep::new("Run clippy", "cargo clippy"));
        assert_eq!(wf.steps.len(), 2);
        assert_eq!(wf.steps[0].name(), "Run tests");
        assert_eq!(wf.steps[0].command(), "cargo test");
        assert_eq!(wf.steps[0].kind(), StepKind::User);
        assert_eq!(wf.steps[1].name(), "Run clippy");
        assert_eq!(wf.steps[1].command(), "cargo clippy");
    }

    #[test]
    fn normalize_basic() {
        assert_eq!(super::normalize("hello-world"), "hello-world");
        assert_eq!(super::normalize("hello world"), "hello-world");
        assert_eq!(super::normalize("a/b/c"), "a-b-c");
        assert_eq!(super::normalize("test@v1.2.3"), "test-v1.2.3");
        assert_eq!(super::normalize("foo_bar.baz"), "foo_bar.baz");
    }

    #[test]
    fn normalize_preserves_allowed_chars() {
        // All of [a-zA-Z0-9_.-] should be preserved
        let allowed = "abcXYZ019_.-";
        assert_eq!(super::normalize(allowed), allowed);
    }

    #[test]
    fn normalize_replaces_disallowed_chars() {
        assert_eq!(super::normalize("a:b"), "a-b");
        assert_eq!(super::normalize("a b"), "a-b");
        assert_eq!(super::normalize("a!b@c#d$e"), "a-b-c-d-e");
    }
}

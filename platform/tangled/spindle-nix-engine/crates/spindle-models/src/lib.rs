//! Shared model types for `tangled-spindle-nix`.
//!
//! This crate contains the core domain types that are shared across all other crates
//! in the workspace: pipeline/workflow identifiers, status enums, log line types,
//! secret masking, environment variable builders, and the workflow logger abstraction.

pub mod log_line;
pub mod pipeline;
pub mod pipeline_env;
pub mod secret_mask;
pub mod status;
pub mod step;
pub mod unlocked_secret;
pub mod workflow;
pub mod workflow_logger;

pub use log_line::{LogKind, LogLine, StepStatus};
pub use pipeline::{Pipeline, PipelineId};
pub use pipeline_env::{
    PipelineEnvVars, PullRequestTriggerData, PushTriggerData, TriggerKind, TriggerMetadata,
    TriggerRepo,
};
pub use secret_mask::SecretMask;
pub use status::StatusKind;
pub use step::{CloneStep, Step, StepKind, UserStep};
pub use unlocked_secret::UnlockedSecret;
pub use workflow::{Workflow, WorkflowId};
pub use workflow_logger::{FileWorkflowLogger, NullLogger, WorkflowLogger};

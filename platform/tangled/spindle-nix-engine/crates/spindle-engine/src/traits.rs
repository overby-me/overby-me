//! Engine trait definition for workflow execution.
//!
//! Matches the upstream Go `Engine` interface from `engine.go`:
//! ```go
//! type Engine interface {
//!     InitWorkflow(twf tangled.Pipeline_Workflow, tpl tangled.Pipeline) (*Workflow, error)
//!     SetupWorkflow(ctx context.Context, wid WorkflowId, wf *Workflow, wfLogger WorkflowLogger) error
//!     WorkflowTimeout() time.Duration
//!     DestroyWorkflow(ctx context.Context, wid WorkflowId) error
//!     RunStep(ctx context.Context, wid WorkflowId, w *Workflow, idx int, secrets []secrets.UnlockedSecret, wfLogger WorkflowLogger) error
//! }
//! ```

use std::time::Duration;

use async_trait::async_trait;
use spindle_models::{Pipeline, UnlockedSecret, Workflow, WorkflowId, WorkflowLogger};

/// Errors that can occur during engine operations.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The workflow manifest is invalid or unsupported.
    #[error("invalid workflow: {0}")]
    InvalidWorkflow(String),

    /// Failed to set up the execution environment (e.g. `nix build` failure).
    #[error("setup failed: {0}")]
    SetupFailed(String),

    /// A step exited with a non-zero exit code.
    #[error("step failed with exit code {exit_code}: {message}")]
    StepFailed { exit_code: i32, message: String },

    /// The workflow exceeded the configured timeout.
    #[error("workflow timed out after {0:?}")]
    Timeout(Duration),

    /// Failed to tear down the execution environment.
    #[error("destroy failed: {0}")]
    DestroyFailed(String),

    /// An I/O error occurred.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A catch-all for other errors.
    #[error("{0}")]
    Other(String),
}

/// Result type alias for engine operations.
pub type EngineResult<T> = Result<T, EngineError>;

/// Raw pipeline workflow data from the AT Protocol record.
///
/// This is the Rust equivalent of the upstream Go `tangled.Pipeline_Workflow`
/// struct. It contains the raw YAML content and metadata needed to parse
/// a workflow into the internal [`Workflow`] representation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PipelineWorkflow {
    /// The workflow name (from the YAML filename).
    pub name: String,
    /// The engine identifier (e.g. `"nix"`, `"nixery"`).
    pub engine: String,
    /// The raw YAML content of the workflow file.
    pub raw: String,
    /// Clone options from the pipeline record.
    #[serde(default)]
    pub clone: Option<PipelineCloneOpts>,
}

/// Clone options from the pipeline record.
///
/// Matches the upstream Go `Pipeline_CloneOpts` struct.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PipelineCloneOpts {
    /// Fetch depth (0 means default shallow clone with depth 1).
    #[serde(default)]
    pub depth: u32,
    /// Whether to skip cloning entirely.
    #[serde(default)]
    pub skip: bool,
    /// Whether to recurse into submodules.
    #[serde(default)]
    pub submodules: bool,
}

/// The core engine trait for workflow execution.
///
/// Each engine implementation (e.g. `nix`) must implement this trait.
/// The trait is object-safe and async, using `async_trait` for async methods.
///
/// # Lifecycle
///
/// For each workflow in a pipeline, the orchestrator calls:
/// 1. [`init_workflow`](Engine::init_workflow) — Parse the raw pipeline workflow into an internal `Workflow`.
/// 2. [`setup_workflow`](Engine::setup_workflow) — Set up the execution environment (build dependencies, create workspace).
/// 3. [`run_step`](Engine::run_step) — Execute each step sequentially.
/// 4. [`destroy_workflow`](Engine::destroy_workflow) — Tear down the execution environment (clean up workspace).
///
/// [`workflow_timeout`](Engine::workflow_timeout) is called to determine the maximum
/// duration for the entire workflow execution.
#[async_trait]
pub trait Engine: Send + Sync {
    /// Transform an incoming pipeline workflow into the internal [`Workflow`] representation.
    ///
    /// This is where the engine parses the raw YAML, validates the engine field,
    /// extracts dependencies, and constructs the step list (including the system
    /// clone step if applicable).
    ///
    /// # Arguments
    /// * `twf` — The raw pipeline workflow from the AT Protocol record.
    /// * `pipeline` — The parent pipeline (provides repo owner/name context).
    fn init_workflow(&self, twf: PipelineWorkflow, pipeline: &Pipeline) -> EngineResult<Workflow>;

    /// Set up the execution environment for a workflow.
    ///
    /// For the nix engine, this builds the Nix closure from the workflow's
    /// dependencies and creates the workspace directory.
    ///
    /// # Arguments
    /// * `wid` — The unique workflow identifier.
    /// * `workflow` — The parsed workflow (from `init_workflow`).
    /// * `logger` — Logger for streaming setup output to clients.
    async fn setup_workflow(
        &self,
        wid: &WorkflowId,
        workflow: &Workflow,
        logger: &dyn WorkflowLogger,
    ) -> EngineResult<()>;

    /// Return the configured workflow timeout.
    ///
    /// If a workflow's total execution time (setup + all steps) exceeds this
    /// duration, it will be cancelled with a `Timeout` status.
    fn workflow_timeout(&self) -> Duration;

    /// Tear down the execution environment for a workflow.
    ///
    /// Cleans up the workspace directory and any other resources allocated
    /// during `setup_workflow`. Called regardless of whether the workflow
    /// succeeded or failed.
    ///
    /// # Arguments
    /// * `wid` — The unique workflow identifier.
    async fn destroy_workflow(&self, wid: &WorkflowId) -> EngineResult<()>;

    /// Execute a single step within the workflow's environment.
    ///
    /// The step is run as a child process with:
    /// - Working directory set to the workspace.
    /// - `PATH` set to include the Nix closure's `bin`/`sbin` directories.
    /// - Environment variables from the pipeline trigger metadata.
    /// - Secrets injected as environment variables (and masked in logs).
    ///
    /// Stdout and stderr are streamed line-by-line to the logger.
    ///
    /// # Arguments
    /// * `wid` — The unique workflow identifier.
    /// * `workflow` — The parsed workflow.
    /// * `step_idx` — Zero-based index of the step to execute.
    /// * `secrets` — Decrypted secrets to inject as environment variables.
    /// * `logger` — Logger for streaming step output to clients.
    async fn run_step(
        &self,
        wid: &WorkflowId,
        workflow: &Workflow,
        step_idx: usize,
        secrets: &[UnlockedSecret],
        logger: &dyn WorkflowLogger,
    ) -> EngineResult<()>;
}

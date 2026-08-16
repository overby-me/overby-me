//! Pipeline orchestration — processing pipeline events and executing workflows.
//!
//! This module is the heart of Phase 6. It:
//! 1. Receives `PipelineEvent`s from the knot consumer channel.
//! 2. Parses the pipeline record, validates ownership, and maps workflows to the engine.
//! 3. Builds pipeline environment variables from trigger metadata.
//! 4. Enqueues a job to the bounded queue for each pipeline.
//! 5. Executes all workflows in parallel within each job:
//!    `setup_workflow` → run steps sequentially → `destroy_workflow`.
//! 6. Manages status transitions and log streaming.

use std::io::Write;
use std::sync::Arc;

use spindle_db::Database;
use spindle_engine::traits::{EngineError, PipelineCloneOpts, PipelineWorkflow};
use spindle_engine::{Engine, NixEngine};
use spindle_knot::{PipelineEvent, PipelineRecord};
use spindle_models::{
    FileWorkflowLogger, Pipeline, PipelineEnvVars, PipelineId, StatusKind, TriggerMetadata,
    WorkflowId, WorkflowLogger,
};
use spindle_queue::JobQueue;
use spindle_secrets::Manager as SecretsManager;
use tracing::{error, info, warn};

use crate::notifier::Notifier;

// ---------------------------------------------------------------------------
// KnotSubscriber newtype wrapper
// ---------------------------------------------------------------------------

/// Newtype wrapper around [`KnotConsumer`] that implements [`KnotSubscriber`].
///
/// Required because both `KnotSubscriber` and `KnotConsumer` are defined in
/// external crates (orphan rule).
pub struct KnotSubscriberAdapter(pub Arc<spindle_knot::KnotConsumer>);

#[async_trait::async_trait]
impl spindle_jetstream::ingester::KnotSubscriber for KnotSubscriberAdapter {
    async fn subscribe(&self, knot: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.0.subscribe(knot).await.map_err(|e| Box::new(e) as _)
    }

    async fn unsubscribe(
        &self,
        knot: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.0.unsubscribe(knot).await.map_err(|e| Box::new(e) as _)
    }
}

/// Shared context for the pipeline orchestrator.
#[allow(dead_code)]
pub struct OrchestratorContext {
    /// Database handle.
    pub db: Arc<Database>,
    /// The Nix engine for workflow execution.
    pub engine: Arc<NixEngine>,
    /// Secrets manager.
    pub secrets: Arc<dyn SecretsManager + Send + Sync>,
    /// Job queue.
    pub queue: Arc<JobQueue>,
    /// Event notifier (broadcast channel for WebSocket clients).
    pub notifier: Arc<Notifier>,
    /// This spindle's hostname.
    pub hostname: String,
    /// This spindle's `did:web:{hostname}`.
    pub did_web: String,
    /// Directory for workflow log files.
    pub log_dir: std::path::PathBuf,
    /// Whether dev mode is enabled.
    pub dev: bool,
}

/// Process a pipeline event received from a knot consumer.
///
/// Parses the event, validates it, creates workflows, and submits a job
/// to the queue for execution.
pub fn process_pipeline_event(ctx: Arc<OrchestratorContext>, event: PipelineEvent) {
    let knot = event.knot.clone();
    let rkey = event.rkey.clone().unwrap_or_default();

    // Parse the pipeline record from the event payload.
    let record: PipelineRecord = match serde_json::from_value(event.payload.clone()) {
        Ok(r) => r,
        Err(e) => {
            warn!(%e, knot = %knot, "failed to parse pipeline record from event");
            return;
        }
    };

    let pipeline_id = PipelineId {
        knot: knot.clone(),
        rkey: rkey.clone(),
    };

    // Extract repo info from the record's trigger metadata.
    let repo_did = match record.repo_did() {
        Some(d) => d.to_string(),
        None => {
            // Fall back to event-level DID.
            match &event.did {
                Some(d) => d.clone(),
                None => {
                    warn!(pipeline = %pipeline_id, "pipeline event missing repo DID");
                    return;
                }
            }
        }
    };

    let repo_name = match record.repo_name() {
        Some(n) => n.to_string(),
        None => {
            warn!(pipeline = %pipeline_id, "pipeline event missing repo name");
            return;
        }
    };

    info!(
        pipeline = %pipeline_id,
        repo_did = %repo_did,
        repo_name = %repo_name,
        "processing pipeline event"
    );

    // Verify the repo is registered on this spindle instance.
    match ctx.db.get_repo(&repo_did, &repo_name) {
        Ok(Some(_)) => {} // Repo is tracked — proceed.
        Ok(None) => {
            warn!(
                pipeline = %pipeline_id,
                repo_did = %repo_did,
                repo_name = %repo_name,
                "rejecting pipeline event: repo not registered on this spindle"
            );
            return;
        }
        Err(e) => {
            error!(
                %e,
                pipeline = %pipeline_id,
                "failed to check repo registration, rejecting pipeline event"
            );
            return;
        }
    }

    // Extract workflows from the record.
    let workflow_manifests = match &record.workflows {
        Some(wfs) if !wfs.is_empty() => wfs.clone(),
        _ => {
            warn!(pipeline = %pipeline_id, "pipeline event has no workflows");
            return;
        }
    };

    // Parse trigger metadata by re-serializing the event payload's triggerMetadata.
    let trigger: Option<TriggerMetadata> = event
        .payload
        .get("triggerMetadata")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    // Build pipeline environment variables.
    let pipeline_env = PipelineEnvVars::build(trigger.as_ref(), &pipeline_id, ctx.dev);

    // Extract commit SHA from trigger metadata for the clone step.
    let commit_sha = trigger.as_ref().and_then(|t| {
        t.push
            .as_ref()
            .map(|p| p.new_sha.clone())
            .or_else(|| t.pull_request.as_ref().map(|pr| pr.source_sha.clone()))
    });

    // Create the Pipeline struct.
    let pipeline = Pipeline {
        repo_owner: repo_did.clone(),
        repo_name: repo_name.clone(),
        commit_sha,
        workflows: Vec::new(), // Workflows are processed individually below.
    };

    // Initialize workflows via the engine.
    let mut workflow_entries = Vec::new();

    for manifest in &workflow_manifests {
        let wf_name = manifest.name.as_deref().unwrap_or("unnamed");

        let engine_name = manifest.engine.as_deref().unwrap_or("nix");
        let raw_content = match &manifest.raw {
            Some(c) => c.clone(),
            None => {
                warn!(
                    pipeline = %pipeline_id,
                    workflow = %wf_name,
                    "workflow manifest has no content, skipping"
                );
                continue;
            }
        };

        // Parse clone options from the workflow manifest.
        let clone_opts: Option<PipelineCloneOpts> = manifest
            .clone
            .as_ref()
            .and_then(|c| serde_json::from_value(c.clone()).ok());

        let pipeline_workflow = PipelineWorkflow {
            name: wf_name.to_string(),
            engine: engine_name.to_string(),
            raw: raw_content,
            clone: clone_opts,
        };

        match ctx.engine.init_workflow(pipeline_workflow, &pipeline) {
            Ok(workflow) => {
                let wid = WorkflowId::new(pipeline_id.clone(), wf_name);
                workflow_entries.push((wid, workflow));
            }
            Err(e) => {
                warn!(
                    %e,
                    pipeline = %pipeline_id,
                    workflow = %wf_name,
                    "failed to initialize workflow, skipping"
                );
            }
        }
    }

    if workflow_entries.is_empty() {
        warn!(pipeline = %pipeline_id, "no valid workflows in pipeline");
        return;
    }

    // Register all workflows as pending in the database.
    for (wid, _) in &workflow_entries {
        let wid_str = wid.to_string();
        if let Err(e) = ctx.db.status_pending(
            &wid_str,
            &pipeline_id.knot,
            &pipeline_id.rkey,
            &repo_did,
            &wid.name,
        ) {
            error!(%e, workflow_id = %wid_str, "failed to create pending status");
        }

        // Emit a status event.
        emit_status_event(
            &ctx,
            &pipeline_id,
            &wid.name,
            StatusKind::Pending,
            None,
            None,
        );
    }

    // Collect workflow IDs before moving entries into the closure.
    let workflow_ids: Vec<WorkflowId> = workflow_entries
        .iter()
        .map(|(wid, _)| wid.clone())
        .collect();

    // Submit a job to execute all workflows.
    let job_ctx = ctx.clone();
    let job_pipeline_id = pipeline_id.clone();
    let job_repo_path = format!("{}/{}", repo_did, repo_name);

    // Compute a pipeline-level timeout as a hard ceiling to guarantee the queue
    // semaphore permit is always released. This is set to the workflow timeout
    // plus generous headroom for setup, cleanup, and destroy_workflow timeouts.
    let pipeline_timeout = ctx.engine.workflow_timeout() + std::time::Duration::from_secs(120);

    let result = ctx.queue.submit(Box::new(move || {
        Box::pin(async move {
            let pipeline_id_for_log = job_pipeline_id.clone();
            let result = tokio::time::timeout(
                pipeline_timeout,
                execute_pipeline(
                    job_ctx,
                    job_pipeline_id,
                    &job_repo_path,
                    workflow_entries,
                    pipeline_env,
                ),
            )
            .await;

            match result {
                Ok(handles) => {
                    // Pipeline completed within timeout. Handles already joined inside.
                    drop(handles);
                }
                Err(_) => {
                    error!(
                        pipeline = %pipeline_id_for_log,
                        timeout = ?pipeline_timeout,
                        "pipeline execution timed out, releasing queue slot"
                    );
                    // Note: spawned workflow tasks are aborted inside
                    // execute_pipeline via the AbortOnDrop wrapper.
                }
            }
        })
    }));

    match result {
        Ok(()) => {
            info!(pipeline = %pipeline_id, "pipeline job submitted to queue");
        }
        Err(e) => {
            error!(%e, pipeline = %pipeline_id, "failed to submit pipeline job to queue");
            // Mark all workflows as failed.
            for wid in &workflow_ids {
                let wid_str = wid.to_string();
                let _ = ctx.db.status_failed(&wid_str);
                emit_status_event(
                    &ctx,
                    &wid.pipeline_id,
                    &wid.name,
                    StatusKind::Failed,
                    None,
                    None,
                );
            }
        }
    }
}

/// A set of JoinHandles that aborts all tasks when dropped.
///
/// This ensures that when a pipeline-level timeout fires and drops the future,
/// all spawned workflow tasks are actually cancelled instead of running forever.
struct AbortOnDrop(Vec<tokio::task::JoinHandle<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        for handle in &self.0 {
            handle.abort();
        }
    }
}

/// Execute all workflows in a pipeline concurrently.
///
/// Spawns workflow tasks and waits for all of them. The `AbortOnDrop` guard
/// lives as a local variable — if the outer timeout drops this future, the
/// guard's `Drop` impl aborts all spawned workflow tasks.
async fn execute_pipeline(
    ctx: Arc<OrchestratorContext>,
    pipeline_id: PipelineId,
    repo_path: &str,
    workflows: Vec<(WorkflowId, spindle_models::Workflow)>,
    pipeline_env: Option<std::collections::HashMap<String, String>>,
) -> AbortOnDrop {
    info!(
        pipeline = %pipeline_id,
        workflow_count = workflows.len(),
        "executing pipeline"
    );

    // Fetch secrets for this repo.
    let secrets = match ctx.secrets.get_secrets_unlocked(repo_path).await {
        Ok(s) => s,
        Err(e) => {
            warn!(%e, repo = %repo_path, "failed to fetch secrets, proceeding without");
            Vec::new()
        }
    };

    // Execute all workflows in parallel.
    let mut handles = Vec::new();

    for (wid, mut workflow) in workflows {
        if let Some(ref env) = pipeline_env {
            for (k, v) in env {
                workflow
                    .environment
                    .entry(k.clone())
                    .or_insert_with(|| v.clone());
            }
        }

        let ctx = ctx.clone();
        let secrets = secrets.clone();

        handles.push(tokio::spawn(async move {
            execute_workflow(ctx, wid, workflow, secrets).await;
        }));
    }

    // Grab abort handles BEFORE awaiting, so we can abort on cancellation.
    let abort_handles: Vec<tokio::task::AbortHandle> =
        handles.iter().map(|h| h.abort_handle()).collect();

    // Guard that aborts all tasks if this future is dropped (e.g. timeout).
    struct AbortGuard(Vec<tokio::task::AbortHandle>);
    impl Drop for AbortGuard {
        fn drop(&mut self) {
            for h in &self.0 {
                h.abort();
            }
        }
    }
    let _guard = AbortGuard(abort_handles);

    // Now await all handles. If the outer timeout drops this future,
    // _guard's Drop fires and aborts all workflow tasks.
    for handle in handles {
        match handle.await {
            Ok(()) => {}
            Err(e) if e.is_cancelled() => {
                warn!(pipeline = %pipeline_id, "workflow task was cancelled");
            }
            Err(e) => {
                error!(%e, pipeline = %pipeline_id, "workflow task panicked");
            }
        }
    }

    info!(pipeline = %pipeline_id, "pipeline execution complete");
    AbortOnDrop(Vec::new())
}

/// Execute a single workflow: setup → run steps → destroy.
async fn execute_workflow(
    ctx: Arc<OrchestratorContext>,
    wid: WorkflowId,
    workflow: spindle_models::Workflow,
    secrets: Vec<spindle_models::UnlockedSecret>,
) {
    let wid_str = wid.to_string();
    let timeout = ctx.engine.workflow_timeout();

    info!(workflow_id = %wid_str, name = %workflow.name, "starting workflow execution");

    // Transition to running.
    if let Err(e) = ctx.db.status_running(&wid_str) {
        error!(%e, workflow_id = %wid_str, "failed to set running status");
    }
    emit_status_event(
        &ctx,
        &wid.pipeline_id,
        &wid.name,
        StatusKind::Running,
        None,
        None,
    );

    // Create the workflow logger.
    let secret_values: Vec<String> = secrets.iter().map(|s| s.value.clone()).collect();
    let logger: Arc<dyn WorkflowLogger> =
        match FileWorkflowLogger::new(&ctx.log_dir, &wid, &secret_values) {
            Ok(l) => Arc::new(l),
            Err(e) => {
                error!(%e, workflow_id = %wid_str, "failed to create workflow logger");
                let _ = ctx.db.status_failed(&wid_str);
                emit_status_event(
                    &ctx,
                    &wid.pipeline_id,
                    &wid.name,
                    StatusKind::Failed,
                    None,
                    None,
                );
                return;
            }
        };

    // Wrap the entire execution in a timeout.
    let result = tokio::time::timeout(timeout, async {
        run_workflow_steps(&ctx, &wid, &workflow, &secrets, logger.as_ref()).await
    })
    .await;

    match result {
        Ok(Ok(())) => {
            // All steps succeeded.
            if let Err(e) = ctx.db.status_success(&wid_str) {
                error!(%e, workflow_id = %wid_str, "failed to set success status");
            }
            emit_status_event(
                &ctx,
                &wid.pipeline_id,
                &wid.name,
                StatusKind::Success,
                None,
                None,
            );
            info!(workflow_id = %wid_str, "workflow completed successfully");
        }
        Ok(Err(e)) => {
            // A step failed.
            error!(%e, workflow_id = %wid_str, "workflow failed");
            if let Err(e) = ctx.db.status_failed(&wid_str) {
                error!(%e, workflow_id = %wid_str, "failed to set failed status");
            }
            emit_status_event(
                &ctx,
                &wid.pipeline_id,
                &wid.name,
                StatusKind::Failed,
                Some(&e.to_string()),
                None,
            );
        }
        Err(_) => {
            // Timed out.
            error!(workflow_id = %wid_str, timeout = ?timeout, "workflow timed out");
            if let Err(e) = ctx.db.status_timeout(&wid_str) {
                error!(%e, workflow_id = %wid_str, "failed to set timeout status");
            }
            emit_status_event(
                &ctx,
                &wid.pipeline_id,
                &wid.name,
                StatusKind::Timeout,
                None,
                None,
            );
        }
    }

    // Always destroy the workflow environment, with a timeout to prevent blocking
    // the queue forever if cleanup hangs.
    let destroy_timeout = std::time::Duration::from_secs(60);
    match tokio::time::timeout(destroy_timeout, ctx.engine.destroy_workflow(&wid)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            warn!(%e, workflow_id = %wid_str, "failed to destroy workflow environment");
        }
        Err(_) => {
            error!(workflow_id = %wid_str, "destroy_workflow timed out after 60s, abandoning cleanup");
        }
    }

    // Close the logger.
    if let Err(e) = logger.close() {
        warn!(%e, workflow_id = %wid_str, "failed to close workflow logger");
    }
}

/// Run setup + all steps for a workflow.
async fn run_workflow_steps(
    ctx: &OrchestratorContext,
    wid: &WorkflowId,
    workflow: &spindle_models::Workflow,
    secrets: &[spindle_models::UnlockedSecret],
    logger: &dyn WorkflowLogger,
) -> Result<(), EngineError> {
    use spindle_models::log_line::StepStatus;

    // Setup the workflow environment (build Nix closure, create workspace).
    info!(workflow_id = %wid, "setting up workflow environment");
    ctx.engine.setup_workflow(wid, workflow, logger).await?;

    // Execute each step sequentially.
    for (step_idx, step) in workflow.steps.iter().enumerate() {
        info!(
            workflow_id = %wid,
            step_idx,
            name = step.name(),
            "executing step"
        );

        // Write control log: step start.
        let mut start_writer = logger.control_writer(step_idx, step.as_ref(), StepStatus::Start);
        let _ = start_writer.write_all(b"start");

        // Run the step.
        let step_result = ctx
            .engine
            .run_step(wid, workflow, step_idx, secrets, logger)
            .await;

        // Write control log: step end.
        let mut end_writer = logger.control_writer(step_idx, step.as_ref(), StepStatus::End);
        let _ = end_writer.write_all(b"end");

        // Propagate step failures.
        step_result?;

        info!(
            workflow_id = %wid,
            step_idx,
            name = step.name(),
            "step completed successfully"
        );
    }

    Ok(())
}

/// The AT Protocol NSID for pipeline status events.
const PIPELINE_STATUS_NSID: &str = "sh.tangled.pipeline.status";

/// The AT Protocol NSID for the pipeline collection.
const PIPELINE_NSID: &str = "sh.tangled.pipeline";

/// Emit a pipeline status event in the format expected by the tangled appview.
///
/// The appview expects events with:
/// - `nsid`: `"sh.tangled.pipeline.status"`
/// - `rkey`: a unique event identifier
/// - `event`: a `PipelineStatus` record with `$type`, `createdAt`, `pipeline` (AT-URI),
///   `workflow`, `status`, and optional `error`/`exitCode` fields.
fn emit_status_event(
    ctx: &OrchestratorContext,
    pipeline_id: &PipelineId,
    workflow_name: &str,
    status: StatusKind,
    error: Option<&str>,
    exit_code: Option<i64>,
) {
    let now = chrono::Utc::now();
    let created_nanos = now.timestamp_nanos_opt().unwrap_or(0);

    // Build the AT-URI for the pipeline: at://did:web:{knot}/sh.tangled.pipeline/{rkey}
    let pipeline_at_uri = format!(
        "at://did:web:{}/{}/{}",
        pipeline_id.knot, PIPELINE_NSID, pipeline_id.rkey
    );

    // Generate a simple rkey from the nanosecond timestamp.
    let rkey = created_nanos.to_string();

    // Build the PipelineStatus record matching the AT Protocol schema.
    let mut payload = serde_json::json!({
        "$type": PIPELINE_STATUS_NSID,
        "createdAt": now.to_rfc3339(),
        "pipeline": pipeline_at_uri,
        "workflow": workflow_name,
        "status": status,
    });

    if let Some(err) = error {
        payload["error"] = serde_json::Value::String(err.to_string());
    }
    if let Some(code) = exit_code {
        payload["exitCode"] = serde_json::Value::Number(code.into());
    }

    let payload_str = payload.to_string();

    let params = spindle_db::events::InsertEventParams {
        rkey: &rkey,
        nsid: PIPELINE_STATUS_NSID,
        payload: &payload_str,
        created: created_nanos,
    };

    // Insert into the events table.
    match ctx.db.insert_event(&params) {
        Ok(event_id) => {
            // Broadcast to WebSocket clients.
            let event = spindle_db::events::Event {
                id: event_id,
                kind: PIPELINE_STATUS_NSID.into(),
                payload: payload_str,
                created_at: now.to_rfc3339(),
                rkey,
                nsid: PIPELINE_STATUS_NSID.into(),
                created: created_nanos,
            };
            ctx.notifier.notify(event);
        }
        Err(e) => {
            warn!(
                %e,
                workflow = workflow_name,
                pipeline = %pipeline_id,
                "failed to insert status event"
            );
        }
    }
}

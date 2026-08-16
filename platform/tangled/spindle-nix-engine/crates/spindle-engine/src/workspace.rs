//! Per-workflow workspace directory management.
//!
//! Each workflow execution gets an isolated workspace directory where the
//! repository is cloned and steps execute. The workspace persists across
//! all steps within a single workflow (matching Docker's `/tangled/workspace`
//! bind mount in the upstream Go spindle).

use std::path::{Path, PathBuf};

use tracing::{debug, info};

use crate::traits::{EngineError, EngineResult};
use spindle_models::WorkflowId;

/// Manages workspace directories for workflow executions.
#[derive(Debug, Clone)]
pub struct WorkspaceManager {
    /// Root directory for all workspaces (e.g. `/var/lib/tangled-spindle-{name}/workspaces`).
    root: PathBuf,
}

impl WorkspaceManager {
    /// Create a new workspace manager with the given root directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Get the workspace directory path for a workflow.
    ///
    /// Format: `{root}/{workflow_id}/`
    pub fn workspace_dir(&self, wid: &WorkflowId) -> PathBuf {
        self.root.join(wid.to_string())
    }

    /// Create the workspace directory for a workflow.
    ///
    /// Creates all parent directories if they don't exist.
    pub async fn create(&self, wid: &WorkflowId) -> EngineResult<PathBuf> {
        let dir = self.workspace_dir(wid);
        tokio::fs::create_dir_all(&dir).await.map_err(|e| {
            EngineError::SetupFailed(format!(
                "failed to create workspace dir {}: {e}",
                dir.display()
            ))
        })?;

        info!(?dir, %wid, "created workspace directory");
        Ok(dir)
    }

    /// Destroy the workspace directory for a workflow.
    ///
    /// Removes the entire workspace directory tree. Errors are logged but
    /// not propagated (best-effort cleanup).
    pub async fn destroy(&self, wid: &WorkflowId) -> EngineResult<()> {
        let dir = self.workspace_dir(wid);
        if dir.exists() {
            debug!(?dir, %wid, "destroying workspace directory");
            tokio::fs::remove_dir_all(&dir).await.map_err(|e| {
                EngineError::DestroyFailed(format!(
                    "failed to remove workspace dir {}: {e}",
                    dir.display()
                ))
            })?;
            info!(?dir, %wid, "destroyed workspace directory");
        }
        Ok(())
    }

    /// Return the root directory for all workspaces.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spindle_models::PipelineId;
    use std::fs;

    fn test_wid() -> WorkflowId {
        WorkflowId::new(
            PipelineId {
                knot: "example.com".into(),
                rkey: "abc123".into(),
            },
            "test",
        )
    }

    fn tempdir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "spindle-ws-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn workspace_dir_path() {
        let mgr = WorkspaceManager::new("/var/lib/spindle/workspaces");
        let wid = test_wid();
        let dir = mgr.workspace_dir(&wid);
        assert_eq!(
            dir,
            PathBuf::from("/var/lib/spindle/workspaces/example.com-abc123-test")
        );
    }

    #[tokio::test]
    async fn create_and_destroy_workspace() {
        let root = tempdir();
        let mgr = WorkspaceManager::new(&root);
        let wid = test_wid();

        let dir = mgr.create(&wid).await.unwrap();
        assert!(dir.exists());
        assert!(dir.is_dir());

        mgr.destroy(&wid).await.unwrap();
        assert!(!dir.exists());

        // Cleanup
        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn destroy_nonexistent_is_ok() {
        let root = tempdir();
        let mgr = WorkspaceManager::new(&root);
        let wid = test_wid();

        // Should not error when workspace doesn't exist.
        mgr.destroy(&wid).await.unwrap();

        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn create_creates_parent_dirs() {
        let root = std::env::temp_dir().join(format!(
            "spindle-ws-nested-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        // root doesn't exist yet
        let mgr = WorkspaceManager::new(&root);
        let wid = test_wid();

        let dir = mgr.create(&wid).await.unwrap();
        assert!(dir.exists());

        let _ = fs::remove_dir_all(&root);
    }
}

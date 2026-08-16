-- Add repo_did column to workflow_status for per-repo filtering.
ALTER TABLE workflow_status ADD COLUMN repo_did TEXT NOT NULL DEFAULT '';

CREATE INDEX IF NOT EXISTS idx_workflow_status_repo_did ON workflow_status(repo_did);

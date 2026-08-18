-- Initial schema for tangled-spindle-nix
-- Matches the upstream Go spindle's SQLite tables.

-- Repos tracked by this spindle instance.
-- When a `sh.tangled.repo` record points at this spindle's hostname,
-- the repo is added here and the spindle subscribes to its knot for pipeline events.
CREATE TABLE IF NOT EXISTS repos (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    did         TEXT NOT NULL,              -- DID of the repo owner (e.g. "did:plc:abc123")
    name        TEXT NOT NULL,              -- Repository name
    knot        TEXT NOT NULL,              -- Knot server hostname
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(did, name)
);

CREATE INDEX IF NOT EXISTS idx_repos_did ON repos(did);
CREATE INDEX IF NOT EXISTS idx_repos_knot ON repos(knot);

-- Spindle members: DIDs that are allowed to trigger pipelines on this spindle.
-- Populated from `sh.tangled.spindle.member` records via Jetstream ingestion.
CREATE TABLE IF NOT EXISTS spindle_members (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    did         TEXT NOT NULL UNIQUE,       -- DID of the member
    role        TEXT NOT NULL DEFAULT 'member',  -- "owner" or "member"
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_spindle_members_did ON spindle_members(did);

-- DIDs to watch on the Jetstream.
-- The Jetstream client subscribes to events for these DIDs.
-- Includes the spindle owner + all members.
CREATE TABLE IF NOT EXISTS dids (
    did         TEXT PRIMARY KEY NOT NULL,
    added_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Pipeline events log.
-- Stored as JSON blobs for WebSocket `/events` backfill.
-- Clients connect with a cursor and receive all events after that cursor.
CREATE TABLE IF NOT EXISTS events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    kind        TEXT NOT NULL,              -- Event kind (e.g. "pipeline_status")
    payload     TEXT NOT NULL,              -- JSON-encoded event payload
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_events_id ON events(id);

-- Workflow execution status tracking.
-- One row per workflow execution, updated as the workflow progresses
-- through its lifecycle: pending → running → success/failed/timeout/cancelled.
CREATE TABLE IF NOT EXISTS workflow_status (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    workflow_id     TEXT NOT NULL,          -- Normalized workflow ID string
    pipeline_knot   TEXT NOT NULL,          -- Knot server hostname
    pipeline_rkey   TEXT NOT NULL,          -- Pipeline record key
    workflow_name   TEXT NOT NULL,          -- Workflow name (from YAML filename)
    status          TEXT NOT NULL DEFAULT 'pending',  -- StatusKind value
    started_at      TEXT,                   -- When the workflow started running
    finished_at     TEXT,                   -- When the workflow reached a terminal state
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(workflow_id)
);

CREATE INDEX IF NOT EXISTS idx_workflow_status_workflow_id ON workflow_status(workflow_id);
CREATE INDEX IF NOT EXISTS idx_workflow_status_pipeline ON workflow_status(pipeline_knot, pipeline_rkey);
CREATE INDEX IF NOT EXISTS idx_workflow_status_status ON workflow_status(status);

-- Cursor persistence for the Jetstream consumer.
-- Stores the last processed event timestamp (in microseconds) so the
-- consumer can resume from where it left off after a restart.
CREATE TABLE IF NOT EXISTS last_time_us (
    id          INTEGER PRIMARY KEY CHECK (id = 1),  -- Singleton row
    time_us     INTEGER NOT NULL DEFAULT 0
);

-- Insert the singleton row if it doesn't exist.
INSERT OR IGNORE INTO last_time_us (id, time_us) VALUES (1, 0);

-- Knot server tracking.
-- Maps knot hostnames to their event stream endpoints.
-- Used by the knot event consumer to know which knots to subscribe to.
CREATE TABLE IF NOT EXISTS knots (
    knot        TEXT PRIMARY KEY NOT NULL,  -- Knot server hostname
    cursor      TEXT,                       -- Last processed event cursor for this knot
    added_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

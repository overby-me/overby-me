-- Add columns needed for appview-compatible event streaming.
--
-- The tangled appview expects events in the format:
--   {"rkey": "...", "nsid": "sh.tangled.pipeline.status", "event": {...}}
--
-- Previously events used `kind` (e.g. "workflow_status") and `id` as cursor.
-- The appview uses `nsid` for filtering and unix-nanos `created` as cursor.

ALTER TABLE events ADD COLUMN rkey TEXT NOT NULL DEFAULT '';
ALTER TABLE events ADD COLUMN nsid TEXT NOT NULL DEFAULT '';
ALTER TABLE events ADD COLUMN created INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_events_created ON events(created);

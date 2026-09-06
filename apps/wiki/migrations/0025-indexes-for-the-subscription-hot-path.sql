-- 0025: the two indexes the per-second subscription polling was missing.
--
-- APPLIED to production on 2026-09-06.
--
-- Found while diagnosing the HB1 weekend outage (2026-09-04/05): Hasura polls
-- every live-query cohort once per second, and two shapes inside that loop had
-- no usable index. Neither matters at one probe; both matter at (subscribers x
-- candidate rows x 1/s), which is what an HB weekend is.
--
-- ONE, the select-permission arm of the nodes RLS. Every visibility check may
-- run this EXISTS per candidate row:
--
--   ... from permissions where context_id = ? and mime_id = ?
--       and active and role = ? and "select"
--
-- The only composite on the table leads (active, INSERT, role, ...): it was
-- built for the insert-permission probe, and the select probe cannot get past
-- the unconstrained `insert` column. The planner sequential-scanned instead,
-- 3,900 times in the 13 quiet hours after Friday's restart, reading all 731
-- rows each time (2.85M tuples). Measured on production:
--
--   before   Seq Scan          22 buffers   0.185 ms   (730 rows filtered away)
--   after    Index Only Scan    4 buffers   0.098 ms
--
-- Partial on (active and "select") so the index holds only rows that can ever
-- satisfy the probe, and equality columns in probe order.
--
-- TWO, the nodes_stream cursor. Stream subscriptions poll
--
--   ... from nodes where updated_at > $cursor ... order by updated_at asc
--
-- once per second per cohort. No index led with updated_at; the best the
-- planner found was nodes_owner_mime_context_updated, where updated_at is the
-- fourth column, so every poll skimmed the whole index to find nothing (the
-- common case), then sorted. Measured on production, empty 5-minute window:
--
--   before   full index skim + Sort   53 buffers   0.198 ms
--   after    Index Scan, no Sort       2 buffers   0.025 ms
--
-- Write cost: none that is not already paid. Two existing indexes contain
-- updated_at, so every edit already forgoes HOT and touches them; one more
-- small btree rides along.
--
-- Not CONCURRENTLY: Hasura's run_sql wraps statements in a transaction, and at
-- 731 and 8,504 rows both builds are under 100 ms. On a table of any size, add
-- CONCURRENTLY and run it outside Hasura.

create index if not exists permissions_select_probe
    on permissions (context_id, mime_id, role)
    where active and "select";

create index if not exists nodes_updated_at_stream
    on nodes (updated_at, id);

-- To undo:
--   drop index if exists permissions_select_probe;
--   drop index if exists nodes_updated_at_stream;

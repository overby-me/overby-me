-- 0002: the bin's read side.
--
-- APPLIED to production on 2026-07-30, together with the Hasura metadata below.
--
-- 0001 added the columns a soft delete writes. This adds the only way to read
-- them back: the base table hides anything stamped from every role, so without
-- a separate entity a binned node would be unreachable even by the person who
-- binned it.

create or replace view deleted_nodes as
  select id, name, key, mime_id, path, parent_id, context_id, owner_id,
         deleted_at, deleted_by, deleted_root
    from nodes
   where deleted_at is not null and id = deleted_root;

comment on view deleted_nodes is
  'The bin: one row per delete a user actually asked for (id = deleted_root), '
  'not every row that delete stamped. The base table hides binned rows from '
  'every role, so this view is the only way back to them.';

-- ── Hasura metadata, applied alongside this ───────────────────────────────
-- 1. Track `deleted_nodes` with camelCase root fields and columns, matching the
--    style the rest of the schema uses (custom_name "deletedNodes").
-- 2. A MANUAL object relationship `context` (deleted_nodes.context_id ->
--    nodes.id): a view has no foreign keys, so Hasura cannot infer it, and the
--    permission below needs to traverse it.
-- 3. SELECT for role `user`, filtered to
--      {context: {members: {_and: [{owner: {_eq: true}},
--                                  {node_id: {_eq: X-Hasura-User-Id}}]}}}
--    so only someone who owns the context can see what was binned in it.
--    Verified: the public role cannot see the field at all.
--
-- Verified end to end against production: binning a folder stamped it and its
-- child in one statement, the view listed only the folder, restoring by
-- deleted_root brought both back, and the bin went empty.

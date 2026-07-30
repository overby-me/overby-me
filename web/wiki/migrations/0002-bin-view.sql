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
-- 4. `deleted_at` and `deleted_root` MUST be in the `nodes` SELECT permission
--    for roles `public` and `user`. Hasura builds `nodes_bool_exp` from the
--    role's selectable columns, so a column missing there cannot be filtered on
--    either: binning ("only stamp what is not already stamped") and restoring
--    (where deleted_root = the id asked for) both failed with
--    "field 'deleted_at' not found in type: 'nodes_bool_exp'". Granting SELECT
--    leaks nothing, because the row filter still hides every stamped row, so
--    the two columns read as null on everything a client can reach.
--
-- Verified end to end against production: binning a folder stamped it and its
-- child in one statement, the view listed only the folder, restoring by
-- deleted_root brought both back, and the bin went empty. Re-verified as the
-- `user` role (impersonated a real context owner) after 4 above, since the
-- first pass ran as admin, which is exactly why the missing columns slipped by.

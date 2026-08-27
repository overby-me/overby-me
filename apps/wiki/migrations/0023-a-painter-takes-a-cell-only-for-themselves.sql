-- 0023: a painter may take a cell over, and may take it only for themselves.
--
-- A METADATA CHANGE, NOT SQL, recorded here for the same reason 0022 is. The
-- query at the end verifies which permission is live.
--
-- WHAT BROKE. The audit that closed the members escalation also removed
-- `owner_id` from the `user` role's updatable columns on `nodes`. Repainting a
-- pixel sets it: Hasura presets the owner on INSERT only, so a repaint that
-- does not set it leaves the cell credited to whoever painted it first. With
-- the column gone the mutation stopped being accepted at all --
--
--     field 'ownerId' not found in type: 'nodes_set_input'
--
-- -- so every repaint on every canvas failed. Reported from production two days
-- later, and only in the browser console, because the paint path asks for the
-- quiet executor so the cooldown does not file a bug report on every early
-- click. That quiet now covers database refusals only; a query the server will
-- not accept is reported like any other fault.
--
-- WHY NOT SIMPLY PUT THE COLUMN BACK. `owner_id` is an unconditional visibility
-- branch (see 0022), so a client that may write it can hand any node it can
-- edit to any account, in or out of the context. It is also the actor the
-- cooldown is charged to (`enforce_rate_limit` trusts `new.owner_id`, on the
-- stated grounds that the INSERT preset makes it unforgeable -- which was never
-- true of an UPDATE). So the column is back, with a check.
--
-- WHAT THE CHECK SAYS. A `canvas/pixel` must end up owned by the session user,
-- which is exactly what painting does and all it can now do. Every other mime
-- is left as it was before the audit, and a context owner keeps every power
-- they have today. Constraining pixels this way makes the canvas STRICTER than
-- it was before the column was removed: a painter can no longer credit a cell
-- to somebody else, nor charge them the cooldown for it.
--
-- The remaining looseness -- a member with per-mime update rights may still
-- reassign a NON-pixel node -- predates all of this and is left alone rather
-- than fixed in a hotfix. Tightening it needs the check to say "owner_id is
-- unchanged", which a Hasura check cannot express, so it wants the write to
-- move behind a function mutation that takes `hasura_session`.
--
-- VERIFIED against production in an isolated sandbox with its own context and
-- no rate limit, driving the app's exact mutation as a plain member:
--
--   * claiming a cell owned by someone else FOR YOURSELF -> affected_rows 1
--   * handing that cell to another account -> "check constraint of an
--     insert/update permission has failed"
--   * editing a DOCUMENT owned by someone else, ownership untouched -> still
--     allowed, so collaborative editing is unaffected
--
-- Sandbox removed afterwards; zero rows left behind.
--
-- HOW TO APPLY: one `bulk` of pg_drop_update_permission + pg_create_update
-- _permission for role `user` on `nodes`, preserving the filter and the other
-- twelve columns. TO REVERT: drop `owner_id` from the columns and set the check
-- back to null, which is the state this replaced.
--
-- THE CHECK NOW LIVE:
--
--     {
--       "_or": [
--         {
--           "owner_id": {
--             "_eq": "X-Hasura-User-Id"
--           }
--         },
--         {
--           "mime_id": {
--             "_neq": "canvas/pixel"
--           }
--         },
--         {
--           "context": {
--             "_or": [
--               {
--                 "owner_id": {
--                   "_eq": "X-Hasura-User-Id"
--                 }
--               },
--               {
--                 "members": {
--                   "_and": [
--                     {
--                       "owner": {
--                         "_eq": true
--                       }
--                     },
--                     {
--                       "node_id": {
--                         "_eq": "X-Hasura-User-Id"
--                       }
--                     }
--                   ]
--                 }
--               }
--             ]
--           }
--         }
--       ]
--     }

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM hdb_catalog.hdb_metadata
        WHERE jsonb_path_exists(
            metadata::jsonb,
            '$.sources[0].tables[*] ? (@.table.name == "nodes")'
            '.update_permissions[*] ? (@.role == "user")'
            '.permission.check._or[*].mime_id._neq ? (@ == "canvas/pixel")'
        )
    ) THEN
        RAISE EXCEPTION 'the pixel ownership check is not live';
    END IF;
END
$$;

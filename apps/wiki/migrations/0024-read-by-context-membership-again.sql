-- 0024: the user role reads by context membership again. 0022 is REVERTED.
--
-- A METADATA CHANGE, recorded as 0022 was, restoring byte-for-byte the filter
-- 0022 quoted as the one it replaced. The query at the end verifies which is
-- live. Everything else 0022 relied on -- 0020's function fix, 0021's rules --
-- is additive and stays.
--
-- WHY. 0022 made the per-mime rows in `permissions` gate READING. Two days
-- later the HB chair reported that only the general secretary and the HB heads
-- could see the folders under HB1 26/27. They were right, and it was this: a
-- member of that context saw 0 of 12 folders, 0 of 6 files and 0 of 15
-- positions, while seeing all 13 policies.
--
-- THE MISTAKE, stated plainly. Those rows are a WRITE model. The proof is in
-- the data, not in an opinion about it:
--
--   * `vote/poll`: 32 contexts define it, ZERO grant members select -- and all
--     32 let members insert `vote/vote` under it. Every context in this wiki
--     expects members to vote in a poll none of them may read. Under 0022 the
--     next poll anyone opened would have been invisible to the room.
--   * `canvas/canvas`: 34 contexts, ONE grants members select. The shared
--     canvas would have gone blank for members in the other 33.
--   * A rule's `role` says who may CREATE that kind of node -- owners make
--     folders, members write policies and comments under them. Reading was
--     always by context membership, which is what the filter this restores
--     asks.
--
-- WHY IT LOOKED SAFE. The sweep behind 0022 measured 304 readers and found
-- nobody losing more than 10 nodes. That was true and it was misleading: it
-- measured a database in which I had, three days earlier, hand-written 103
-- member-read rules over exactly the pairs that would otherwise have gone
-- dark. Every member-read rule on folders, files, documents and positions in
-- this database is `insert=f select=t update=f delete=f` -- 30 of 30 folder
-- rules, 25 of 25 position rules, 18 of 18 document rules -- which is the shape
-- that backfill made. The measurement was taken through a patch and reported
-- as a property of the system.
--
-- HB1 26/27 is simply the first context created after that backfill. It got
-- the template's rules (`nodes.rs::context_permission_objects`), which seed one
-- rule per mime and give the structural mimes to `owner`, so it never received
-- the member-read rows the older contexts had been given. Every context created
-- from that template would have arrived broken the same way.
--
-- WHAT THIS COSTS. The leak 0018 and 0020 named is open again: a signed-in
-- member of a context can read everything in that context, including material
-- whose rule grants only `owner`. Measured at the time, that is 10 poll nodes
-- from two past meetings plus the Test context. Closing it needs a read model
-- that is actually written down -- which mimes a member may see, stated
-- somewhere that says so -- not the write rules reinterpreted.

--     {
--       "_and": [
--         {
--           "_or": [
--             {
--               "owner_id": {
--                 "_eq": "X-Hasura-User-Id"
--               }
--             },
--             {
--               "context": {
--                 "members": {
--                   "node_id": {
--                     "_eq": "X-Hasura-User-Id"
--                   }
--                 }
--               }
--             },
--             {
--               "contextPublicPermissions": {
--                 "active": {
--                   "_eq": true
--                 },
--                 "role": {
--                   "_eq": "public"
--                 },
--                 "select": {
--                   "_eq": true
--                 }
--               }
--             },
--             {
--               "members": {
--                 "emailUser": {
--                   "id": {
--                     "_eq": "X-Hasura-User-Id"
--                   }
--                 }
--               }
--             },
--             {
--               "context": {
--                 "owner_id": {
--                   "_eq": "X-Hasura-User-Id"
--                 }
--               }
--             },
--             {
--               "context": {
--                 "members": {
--                   "emailUser": {
--                     "id": {
--                       "_eq": "X-Hasura-User-Id"
--                     }
--                   }
--                 }
--               }
--             }
--           ]
--         },
--         {
--           "deleted_at": {
--             "_is_null": true
--           }
--         }
--       ]
--     }

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM hdb_catalog.hdb_metadata
        WHERE jsonb_path_exists(
            metadata::jsonb,
            '$.sources[0].tables[*] ? (@.table.name == "nodes")'
            '.select_permissions[*] ? (@.role == "user")'
            '.permission.filter._and[0]._or[*].contextPublicPermissions.role ? (@ == "member")'
        )
    ) THEN
        RAISE EXCEPTION 'the per-mime read filter from 0022 is still live';
    END IF;
END
$$;

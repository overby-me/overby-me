-- 0020: the row-level select check asks whether you own THIS node.
--
-- This is the fix 0018 named and deliberately left alone. Branch 2 read
--
--     permission.role = ANY('{"owner","member"}')
--     AND EXISTS (SELECT 1 FROM nodes AS subnode
--                  WHERE subnode.owner_id = <the session user>)
--
-- with `subnode` unbound, which asks whether the reader owns ANY node rather
-- than whether they own THIS one. It now reads `node.owner_id = <the session
-- user>`, which is what the other three branches already assume it meant.
--
-- WHY THIS IS SAFE NOW, AND WHY IT LOOKED DANGEROUS THEN.
--
-- 0018 estimated the correction "removes 15-37% of what a typical reader can
-- currently see" and called it "not something to ship days before a congress".
-- Measured again before writing this, that estimate is right about the
-- FUNCTION and wrong about the CONSEQUENCE, because the function is not what
-- gates a signed-in reader:
--
--   * `nodes` has exactly two select permissions. The `user` one filters on
--     ownership and context membership directly (owner_id, context.members,
--     contextPublicPermissions, ...) and never calls this function.
--   * The `public` one IS this function: `{"select": {"_eq": "true"}}`. So the
--     only caller is the signed-out reader.
--
-- A signed-out reader has no `x-hasura-user-id`. Branch 2 therefore compares
-- against NULL and is false in BOTH forms: `owner_id = NULL` matches no row,
-- and `node.owner_id = NULL` is NULL. The correction cannot change the answer
-- for the only role that asks.
--
-- VERIFIED BEFORE SWAPPING, the way 0018 did it: the new body was created
-- alongside the old one as `select_candidate` and both were run over every live
-- node (8,238) with an anonymous session. Three nodes visible under each, ZERO
-- disagreements. The candidate function was dropped afterwards.
--
-- WHAT IS STILL TRUE, AND MATTERS IF THIS FUNCTION IS EVER WIRED INTO THE
-- SIGNED-IN PATH. Evaluated for a signed-in reader the difference is large, and
-- mostly a leak being closed. For one plain member (3 memberships, 45 authored
-- nodes) the function currently answers true for 8,157 of 8,238 nodes -- very
-- nearly the whole wiki -- because they own at least one node somewhere. Of the
-- 3,921 that the correction takes away:
--
--   * 3,821 are in contexts they are not a member of: a genuine leak
--   * 100 are in contexts they DO belong to
--
-- Those 100 are the reason 0018 warned about a backfill. Across the whole
-- database the shape is: 7,057 nodes in (context, mime) pairs that already
-- grant `member` select (unaffected); 1,097 nodes in 111 pairs that grant only
-- `owner` (folders, files, positions, documents -- members would lose these);
-- and 81 nodes in 38 pairs with no select permission at all.
--
-- So if the `user` role is ever moved onto this function, it must ship WITH
-- permission rows granting `member` select on those 111 pairs. Doing that today
-- would be granting access nobody is currently using this function to get, so
-- it is left out: this migration changes the function only, and changes no
-- answer any caller receives today.
--
-- HOW TO APPLY. Not through Hasura's `run_sql`: that connects as `nhost_hasura`
-- and this function is owned by `nhost_admin`, which answers "must be owner of
-- function select". It needs the same privileged channel 0018 went through (the
-- nhost SQL editor, or psql as the admin role).
--
-- TO REVERT: re-run 0018's function body, which this file quotes in full at the
-- top of the branch it changes.

CREATE OR REPLACE FUNCTION public."select"(node nodes, hasura_session json)
 RETURNS boolean
 LANGUAGE sql
 STABLE
AS $function$
SELECT EXISTS(
    SELECT 1
    FROM permissions AS permission
    WHERE node.context_id = permission.context_id
      AND node.mime_id = permission.mime_id
      AND permission.active = true
      AND permission.select = true
      AND (
          permission.role = 'public'
          OR (
              -- The fix: this node, not any node.
              permission.role = ANY('{"owner", "member"}')
              AND node.owner_id = (hasura_session->>'x-hasura-user-id')::uuid
          )
          -- Branches 3 and 4 are byte-identical to what 0018 left, `subnode`
          -- join and all. Dropping that join looks equivalent and is not: it
          -- also asserts the node behind `permission.node_id` still exists, and
          -- an anonymous session never reaches these branches, so no test of
          -- the signed-out path could have caught the difference.
          OR (
              permission.role = 'owner'
              AND EXISTS (
                  SELECT 1
                  FROM members AS member, nodes AS subnode
                  WHERE subnode.id = permission.node_id
                    AND subnode.id = member.parent_id
                    AND member.node_id = (hasura_session->>'x-hasura-user-id')::uuid
                    AND member.owner = true
              )
          )
          OR (
              permission.role = 'member'
              AND EXISTS (
                  SELECT 1
                  FROM members AS member, nodes AS subnode
                  WHERE subnode.id = permission.node_id
                    AND subnode.id = member.parent_id
                    AND member.node_id = (hasura_session->>'x-hasura-user-id')::uuid
              )
          )
      )
)
$function$;

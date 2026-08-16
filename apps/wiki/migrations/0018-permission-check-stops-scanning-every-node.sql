-- 0018: the row-level select check stops walking every node to say "no".
--
-- SAME ANSWERS. This migration changes how the check is evaluated and not what
-- it decides. There is a separate, real bug in what it decides -- see the note
-- at the bottom -- and it is deliberately NOT fixed here.
--
-- WHAT WAS WRONG. The body read
--
--     FROM permissions AS permission, nodes AS subnode
--     WHERE node.context_id = permission.context_id ... AND EXISTS ( ... )
--
-- with `subnode` cross-joined and constrained only INSIDE the EXISTS branches.
-- A check that succeeds stops at the first pair that satisfies it, so it is
-- fast. A check that FAILS has to exhaust the product of permissions and nodes
-- before it can return false.
--
-- Measured on production:
--
--   * a failing check costs ~42ms and reads the whole nodes table (20 checks =
--     840ms), against ~0.25ms for one that succeeds
--   * the invitations subscription is mounted in Layout, so every signed-in
--     reader holds one, and it filters members.node_id = that reader's own id
--     -- which makes every reader their own Hasura cohort, re-evaluated on
--     every poll rather than shared with anyone
--   * one reader with 206 memberships, all of which fail the check, took
--     8,139ms for a single evaluation of it (215,401 buffers)
--
-- At the handful of concurrent readers this was measured with, none of that is
-- visible. At the 300 expected this weekend it would not have survived: 300
-- cohorts re-evaluated on a ~1s interval, with a single heavy reader able to
-- exceed the interval on their own.
--
-- WHAT CHANGED. Each branch now binds `subnode` itself instead of leaving it to
-- a cross join:
--
--   * branches 3 and 4 already pinned `subnode.id = permission.node_id`, so
--     they become an EXISTS over members joined to that one node
--   * branch 2 never constrained `subnode` at all, so its unbound existence
--     test moves inside the branch that uses it, unchanged
--   * branch 1 never referenced `subnode`, so it needs nothing
--
-- A failing check is now ~7ms for 20 (from 840ms), and that reader's 8,139ms
-- evaluation is 99.8ms.
--
-- VERIFIED BEFORE SWAPPING, not after: the new body was created alongside the
-- old one and both were run over every node for a reader who owns many nodes, a
-- context owner, a plain member, the 206-membership reader and an id belonging
-- to nobody -- covering true and false results in every branch. Zero
-- disagreements.
--
-- THE BUG THIS DOES NOT FIX. Branch 2 reads
--
--     permission.role = ANY('{"owner","member"}')
--     AND subnode.owner_id = <the session user>
--
-- with `subnode` unbound, which asks whether the reader owns ANY node rather
-- than whether they own THIS one. It is preserved here exactly, bug and all,
-- because this migration is about evaluation cost.
--
-- It is worth fixing, and it is not a small change: correcting it to
-- `node.owner_id` removes 15-37% of what a typical reader can currently see --
-- for one plain member, 111 of 400 sampled nodes, including 28 motions, 20
-- files, 19 folders and 16 amendments. The permission ROWS were evidently
-- written while this loophole was open, so nobody had to grant `member` where
-- content should be readable by members; it simply was. Correcting the function
-- without also correcting the rows would lock readers out of things they need.
-- That is a deliberate piece of work with its own review, not a rider on a
-- performance fix, and not something to ship days before a congress.

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
              -- Preserved exactly as it was, unbound `subnode` and all: this
              -- asks whether the reader owns any node, which is the bug named
              -- above. Changing it belongs to its own migration.
              permission.role = ANY('{"owner", "member"}')
              AND EXISTS (
                  SELECT 1
                  FROM nodes AS subnode
                  WHERE subnode.owner_id = (hasura_session->>'x-hasura-user-id')::uuid
              )
          )
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

-- 0017: the ordinal on a node stops re-sorting its whole sibling set.
--
-- `get_index` is a Hasura COMPUTED FIELD, so it runs once per row returned. It
-- answered "which number is this one" by ranking every sibling with
-- ROW_NUMBER() and then throwing all but one row away -- which makes listing a
-- folder of N children cost N rankings of N rows.
--
-- Measured on production before this change (39h of pg_stat_statements):
--
--   * 38,478 calls, 347s, 9ms mean, and EXPLAIN on the worst parent shows
--     Seq Scan on nodes (973 buffers -- the entire table) -> Sort 4096 rows ->
--     WindowAgg, 6.3ms for ONE call.
--   * One parent has 4,096 of the table's 8,200 rows, so that folder is the
--     shape this is worst for.
--   * It is what makes the two slowest statements in the database 859ms mean /
--     3.4s worst and 538ms / 2.4s: both select getIndex alongside isOwner and
--     isContextOwner over a listing.
--
-- The database is NOT under load -- 4,401s of query time over 39.3h is 3.1% of
-- one core -- so this is about the seconds a reader waits for a big folder, not
-- about capacity.
--
-- COUNTING INSTEAD OF RANKING. The n-th row in an order is the one with n-1
-- rows before it, so the answer is a count of siblings that sort earlier. That
-- is a range over an index rather than a sort of everything, and the index
-- below is built to serve exactly this predicate.
--
-- THE TIE-BREAK IS NEW, and is a fix in its own right. Counting "rows before
-- me" is only equal to ROW_NUMBER when the ordering is total, and
-- (index, updated_at) is not: production has one group -- two `wiki/file` nodes
-- under a95b8d4d-2e94-4f85-86b7-0e2e7cc3bfe4, both index 1, both updated
-- 2025-03-01 12:50:31.299006+00 -- where it is not. ROW_NUMBER hands those two
-- 1 and 2 in whatever order the scan produced, so their numbers could swap
-- between one query and the next. Adding `id` last makes the order total, which
-- makes the count exact AND makes those two numbers stop moving.
--
-- WHEN IT IS NULL. The old body returned NULL whenever the node was not in its
-- own sibling set, which is every node that is mutable (excluded by the WHERE),
-- and every node whose parent_id or mime_id is NULL (nothing equals NULL, so
-- the set came back empty). A plain count would answer 1 for those instead, so
-- the cases are named explicitly rather than left to fall out of the join.

-- Serves the sibling predicate and the range in one index, so the count is an
-- index-only scan. Nothing existed for (parent_id, mime_id, mutable): the
-- planner was choosing a sequential scan, which for the 4,096-child parent is
-- half the table per call.
CREATE INDEX IF NOT EXISTS nodes_siblings_ordinal_idx
    ON public.nodes (parent_id, mime_id, mutable, index, updated_at, id);

CREATE OR REPLACE FUNCTION public.get_index(node nodes)
 RETURNS integer
 LANGUAGE sql
 STABLE
AS $function$
SELECT CASE
    WHEN node.mutable OR node.parent_id IS NULL OR node.mime_id IS NULL THEN NULL
    ELSE (
        SELECT count(*) + 1
        FROM nodes AS sibling
        WHERE sibling.parent_id = node.parent_id
          AND sibling.mime_id = node.mime_id
          AND sibling.mutable = false
          AND (sibling.index, sibling.updated_at, sibling.id)
            < (node.index, node.updated_at, node.id)
    )::int
END
$function$;

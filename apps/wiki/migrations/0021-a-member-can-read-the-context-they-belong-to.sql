-- 0021: every (context, mime) that holds a node gets a member read rule.
--
-- PREREQUISITE FOR THE `select` MIGRATION, NOT A CHANGE OF ITS OWN. Today the
-- `user` role's row filter never consults these rows except to look for a
-- `public` one, so inserting `member` rules changes nothing any reader sees
-- right now. It is what has to exist BEFORE the filter starts consulting them.
--
-- WHY. Every branch of the rule that replaces the filter (see 0020, and the
-- measurement below) begins by requiring an active select permission for the
-- node's (context_id, mime_id). 36 such pairs -- holding 75 live nodes -- have
-- no select rule at all, so under the new filter those nodes would be visible
-- to nobody, including the person who wrote them. Thirty of the 36 are a
-- context's OWN node (`wiki/event`, `wiki/group`, `conference/conference`):
-- without this, a member could not see the event they are a member of, and the
-- context would not render at all.
--
-- SHAPE. Copied from the one context that already does this -- Radikal Ungdom's
-- `wiki/group`/`member` rule -- which is select-only (insert/update/delete
-- false) with `node_id` = the context node, the convention every rule in the
-- table follows. `parents` is the set of mimes the pair's nodes actually hang
-- under, so the rule describes where that mime really lives.
--
-- The Landsmøde 2022 content in the list (31 `vote/policy`, 10 `wiki/folder`,
-- 1 `speak/list`) gets exactly what Landsmøde 2023 through 2026 already grant
-- for the same mimes. It is read-only here because 2022 is a past congress.
--
-- DELIBERATELY EXCLUDED:
--
--   * `wiki/feedback`. One node, one author, and it is a report ABOUT the wiki
--     sent to whoever runs it. Members can read it today only because the
--     current filter ignores per-mime rules; leaving it ruleless means the new
--     filter shows it to its author alone, which is what feedback should be.
--   * Pairs that already grant `owner` select but not `member`. A plain member
--     losing those is the POINT of the migration, not a regression to patch.
--   * Nodes with no `context_id`, and the one node whose `context_id` names a
--     row that no longer exists. No permission row can name either, so the new
--     filter reaches them only through its unconditional owner branch.
--
-- HOW TO APPLY. Rows in an application table, so `nhost_hasura` may write them:
-- Hasura's `run_sql` works, as does psql. Run the whole file: the guard at the
-- end aborts the transaction unless exactly 36 rules were created and no pair
-- ended up with two.
--
-- TO REVERT: delete the rules this created, which are identifiable as the only
-- `member` select-only rules whose `id` is in the set the RETURNING clause
-- prints. Reverting is safe in the same sense applying is: while the `user`
-- filter is still the inline one, neither direction changes what anyone sees.

BEGIN;

CREATE TEMP TABLE created_rules ON COMMIT DROP AS
WITH gap AS (
    SELECT n.context_id,
           n.mime_id,
           array_agg(DISTINCT parent.mime_id) FILTER (WHERE parent.mime_id IS NOT NULL) AS parents
    FROM nodes AS n
    LEFT JOIN nodes AS parent ON parent.id = n.parent_id
    WHERE n.deleted_at IS NULL
      AND n.context_id IS NOT NULL
      AND n.mime_id IS NOT NULL
      AND n.mime_id <> 'wiki/feedback'
      -- One live node names a context row that is gone. `permissions` has a
      -- foreign key to `nodes`, so a rule for it cannot exist to be written.
      AND EXISTS (SELECT 1 FROM nodes AS ctx WHERE ctx.id = n.context_id)
      AND NOT EXISTS (
          SELECT 1
          FROM permissions AS p
          WHERE p.context_id = n.context_id
            AND p.mime_id = n.mime_id
            AND p.active
            AND p."select"
      )
    GROUP BY n.context_id, n.mime_id
), made AS (
    INSERT INTO permissions
        (id, "insert", "select", "update", "delete", node_id, active, context_id, parents, mime_id, role)
    SELECT gen_random_uuid(), false, true, false, false,
           gap.context_id, true, gap.context_id, gap.parents, gap.mime_id, 'member'
    FROM gap
    RETURNING id, context_id, mime_id
)
SELECT * FROM made;

DO $$
DECLARE
    made integer;
    dupes integer;
BEGIN
    SELECT count(*) INTO made FROM created_rules;
    IF made <> 36 THEN
        RAISE EXCEPTION 'expected 36 new rules, created %', made;
    END IF;

    SELECT count(*) INTO dupes
    FROM (
        SELECT p.context_id, p.mime_id
        FROM permissions AS p
        JOIN created_rules AS c USING (context_id, mime_id)
        WHERE p.active AND p."select" AND p.role = 'member'
        GROUP BY p.context_id, p.mime_id
        HAVING count(*) > 1
    ) AS d;
    IF dupes <> 0 THEN
        RAISE EXCEPTION '% pairs ended up with more than one member select rule', dupes;
    END IF;

    IF EXISTS (SELECT 1 FROM created_rules WHERE mime_id = 'wiki/feedback') THEN
        RAISE EXCEPTION 'wiki/feedback is meant to stay ruleless';
    END IF;
END
$$;

COMMIT;

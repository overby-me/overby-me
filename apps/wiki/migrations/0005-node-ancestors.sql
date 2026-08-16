-- 0005: every node's line of descent, as ids.
--
-- APPLIED to production on 2026-07-30.
--
-- `path` (0001) already describes where a node sits, and a prefix match on it
-- answers "everything under here". Two things it cannot do:
--
-- 1. A group's feed. A group holds events, an event's content carries the
--    EVENT as its context, so the group's feed — filtered on context_id — was
--    blind to everything that actually happens in the group. Rolling it up
--    needs a subtree test, and the group is not the parent of that content.
-- 2. Match exactly. Keys contain underscores ('radikal_ungdom'), and `_` is a
--    LIKE wildcard, so `path like 'radikal_ungdom/%'` also matches a sibling
--    called `radikalXungdom`. Harmless so far by luck of the naming, not by
--    construction, and it is the predicate the bin deletes with.
--
-- So: the ancestor ids, root first, excluding the node itself. Containment is
-- exact, needs no escaping, and survives a rename — which `path` does not, and
-- which matters here because a feed query outlives the name it was written
-- against.
--
-- Additive: old.radikal.wiki never names this column.

begin;

alter table nodes add column if not exists ancestors uuid[];

comment on column nodes.ancestors is
  'Ids from the root down to this node''s parent, root first, excluding the '
  'node itself. Maintained by trigger alongside `path`; parent_id remains the '
  'source of truth. Null where `path` is null (an orphan): both mean the same '
  'thing, that this node is not reachable from the root.';

-- ── backfill, BEFORE the trigger exists ──────────────────────────────────
-- One statement for the whole table, for the same reason 0001 backfills first:
-- with the cascade installed, each row written here would fire a subtree update
-- that this statement has already done.
with recursive t as (
  select id, '{}'::uuid[] as a
    from nodes
   where parent_id is null
  union all
  select n.id, t.a || n.parent_id
    from nodes n
    join t on n.parent_id = t.id
)
update nodes n set ancestors = t.a from t
 where t.id = n.id and n.ancestors is distinct from t.a;

-- ── maintenance ──────────────────────────────────────────────────────────
-- Deliberately a SEPARATE trigger pair from 0001's rather than an edit of it:
-- this migration has to be runnable on a database that already has 0001, and a
-- rewritten function there would be invisible to anyone reading only this file.
create or replace function nodes_set_ancestors() returns trigger as $$
declare
  parent_ancestors uuid[];
begin
  if new.parent_id is null then
    new.ancestors := '{}'::uuid[];        -- the root descends from nothing
  else
    select p.ancestors into parent_ancestors from nodes p where p.id = new.parent_id;
    if parent_ancestors is null then
      new.ancestors := null;              -- parent missing or not yet computed
    else
      new.ancestors := parent_ancestors || new.parent_id;
    end if;
  end if;
  return new;
end;
$$ language plpgsql;

drop trigger if exists nodes_ancestors_before on nodes;
create trigger nodes_ancestors_before
  before insert or update of parent_id on nodes
  for each row execute function nodes_set_ancestors();

-- A MOVE rewrites the subtree beneath it. A rename does not: that is the whole
-- point of holding ids rather than keys, and it is why this fires on a change of
-- `ancestors` where the path trigger has to watch `key` as well.
create or replace function nodes_cascade_ancestors() returns trigger as $$
begin
  if new.ancestors is distinct from old.ancestors and new.ancestors is not null then
    -- A descendant's line reads: <old line of the moved node> <the moved node>
    -- <whatever is below it>. Only the first part changes, so keep the tail from
    -- one past the moved node's own position.
    --
    -- coalesce, because array_length of an EMPTY array is null, not 0 — moving a
    -- node that hung directly off the root would otherwise null every line
    -- beneath it.
    update nodes
       set ancestors = new.ancestors || new.id ||
                       ancestors[coalesce(array_length(old.ancestors, 1), 0) + 2
                                 : coalesce(array_length(ancestors, 1), 0)]
     where ancestors @> array[new.id];
  end if;
  return null;
end;
$$ language plpgsql;

drop trigger if exists nodes_ancestors_after on nodes;
create trigger nodes_ancestors_after
  after update on nodes
  for each row
  -- Same two guards as the path cascade, for the same two reasons: the column is
  -- set by the BEFORE trigger rather than named in the statement, and each row
  -- the cascade writes would otherwise re-enter here to redo finished work.
  when (new.ancestors is distinct from old.ancestors and pg_trigger_depth() = 0)
  execute function nodes_cascade_ancestors();

-- ── index ────────────────────────────────────────────────────────────────
-- GIN, because every query against this column is a containment test.
create index if not exists nodes_ancestors_gin on nodes using gin (ancestors);

commit;

-- ── Hasura metadata, applied alongside this ──────────────────────────────
-- `ancestors` added to the `nodes` SELECT permission for roles `public` and
-- `user`. As with 0002's columns, a column absent from a role's selectable set
-- cannot be FILTERED on either, and filtering is the only thing this is for.
-- It discloses nothing a reader cannot already see: it is a list of ids of
-- nodes above this one, and the row rules still decide which of those rows can
-- be read.
--
-- Verified against production: the backfill matches a recursive walk on all
-- 3971 rows (527 nulls, the same orphans 0001 found); an insert three deep, a
-- move of a subtree and a rename all leave it exact, with the rename changing
-- `path` and deliberately NOT `ancestors`. The group feed, replayed as a real
-- member with the app's own operation, went from five stale amendment rows to
-- the meetings that actually happened.
--
-- ── consistency check (zero rows means correct) ──────────────────────────
-- with recursive t as (
--   select id, '{}'::uuid[] as a from nodes where parent_id is null
--   union all
--   select n.id, t.a || n.parent_id from nodes n join t on n.parent_id = t.id
-- )
-- select n.id from nodes n join t on t.id = n.id
--  where n.ancestors is distinct from t.a;

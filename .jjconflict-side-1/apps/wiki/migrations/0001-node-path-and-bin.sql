-- 0001: node paths, and the columns a bin is built on.
--
-- APPLIED to production on 2026-07-30. Kept as the record of what was run and
-- as the thing to re-run against any other copy of this database.
--
-- Additive and backward compatible: old.radikal.wiki names none of these
-- columns, so it cannot see them, and nothing changes for either app until a
-- client writes `deleted_at`.
--
-- Everything is idempotent, so a partial run can be repeated. The Hasura
-- metadata that goes with it is NOT SQL and is listed at the end.

begin;

-- ── columns ──────────────────────────────────────────────────────────────
alter table nodes
  add column if not exists path         text,
  add column if not exists deleted_at   timestamptz,
  add column if not exists deleted_by   uuid,
  -- The node whose deletion the user actually asked for. Every row stamped by
  -- that one action carries it, which is what makes restore exact: it undoes an
  -- ACTION rather than guessing at a tree shape that may have changed since.
  add column if not exists deleted_root uuid;

comment on column nodes.path is
  'Slash-joined keys from the root, exclusive of the root itself '
  '(''ru/lm2026/dagsorden''). Maintained by trigger; parent_id stays the source '
  'of truth and this is a cache of it. Null for a node whose parent row is '
  'missing, which is itself a useful signal.';
comment on column nodes.deleted_at is
  'Set instead of deleting. Hidden from every client by the select rule; '
  'restorable from the bin.';

-- ── backfill, BEFORE the triggers exist ──────────────────────────────────
-- Deliberately first: this writes `path` on every row, and doing it with the
-- cascade trigger installed would fire that trigger once per row, each firing a
-- subtree update that the single statement below has already done.
with recursive t as (
  select id, ''::text as p
    from nodes
   where parent_id is null
  union all
  select n.id, case when t.p = '' then n.key else t.p || '/' || n.key end
    from nodes n
    join t on n.parent_id = t.id
)
update nodes n set path = t.p from t where t.id = n.id and n.path is distinct from t.p;
-- Rows unreachable from the root (orphans and their descendants) keep a null
-- path. There were 527 of them at the time of writing, under 279 missing
-- parents, which is the orphan problem this does not attempt to solve.

-- ── path maintenance ─────────────────────────────────────────────────────
-- Never raises: a write that failed because a denormalised cache could not be
-- computed would be a far worse bug than a null path.
create or replace function nodes_set_path() returns trigger as $$
declare
  parent_path text;
begin
  if new.parent_id is null then
    new.path := '';                       -- the root; its children are unprefixed
  else
    select p.path into parent_path from nodes p where p.id = new.parent_id;
    if parent_path is null then
      new.path := null;                   -- parent missing or not yet computed
    elsif parent_path = '' then
      new.path := new.key;
    else
      new.path := parent_path || '/' || new.key;
    end if;
  end if;
  return new;
end;
$$ language plpgsql;

drop trigger if exists nodes_path_before on nodes;
create trigger nodes_path_before
  before insert or update of parent_id, key on nodes
  for each row execute function nodes_set_path();

-- A move or a rename rewrites the whole subtree beneath it, in one statement,
-- because the column indexes itself by prefix.
create or replace function nodes_cascade_path() returns trigger as $$
begin
  if new.path is distinct from old.path
     and old.path is not null and old.path <> '' then
    update nodes
       set path = new.path || substr(path, length(old.path) + 1)
     where path like old.path || '/%';
  end if;
  return null;
end;
$$ language plpgsql;

drop trigger if exists nodes_path_after on nodes;
create trigger nodes_path_after
  after update on nodes
  for each row
  -- NOT `after update of path`: that fires only when `path` is named in the
  -- statement's SET list, and a rename sets `key`. The BEFORE trigger changing
  -- NEW.path does not count, so the first version of this cascaded nothing.
  --
  -- pg_trigger_depth() = 0 restricts it to top-level statements: the cascade
  -- above writes `path` on each descendant, and without this each of those
  -- writes would re-enter here and redo work already done.
  when (new.path is distinct from old.path and pg_trigger_depth() = 0)
  execute function nodes_cascade_path();

-- ── indexes ──────────────────────────────────────────────────────────────
create index if not exists nodes_path_prefix_idx on nodes (path text_pattern_ops);

create unique index if not exists nodes_path_live_idx
  on nodes (path) where deleted_at is null and path is not null;

-- (parent_id, key) was an UNCONDITIONAL unique constraint, which would have made
-- restoring from the bin impossible: a binned node keeps its key, so anything
-- created in its place would block it coming back. Swapped for the partial form.
--
-- Deliberately keeping the constraint's NAME: components/vote/poll.rs matches on
-- it in an error string to tell a duplicate apart from a real failure, and
-- Postgres reports the index name in that message.
do $$
begin
  if exists (select 1 from pg_constraint
              where conrelid = 'public.nodes'::regclass
                and conname = 'nodes_parent_id_namespace_key') then
    alter table nodes drop constraint nodes_parent_id_namespace_key;
    create unique index nodes_parent_id_namespace_key
      on nodes (parent_id, key) where deleted_at is null;
  end if;
end $$;

commit;

-- ── consistency check (run any time; zero rows means correct) ─────────────
-- Verified zero across all 3975 rows after applying, and again after a rename
-- and a move test that cascaded 150 descendants each.
--
-- with recursive t as (
--   select id, ''::text as p from nodes where parent_id is null
--   union all
--   select n.id, case when t.p = '' then n.key else t.p || '/' || n.key end
--     from nodes n join t on n.parent_id = t.id
-- )
-- select n.id, n.path as stored, t.p as computed
--   from nodes n join t using (id)
--  where n.path is distinct from t.p;

-- ── Hasura metadata, applied alongside this ───────────────────────────────
-- 1. reload_metadata with reload_sources, so the new columns are known.
-- 2. SELECT on `nodes`, roles `public` and `user`: filter wrapped as
--    {_and: [<existing>, {deleted_at: {_is_null: true}}]}, and `path` added to
--    the columns. This is what makes a binned node invisible to BOTH apps with
--    no client change. Verified by binning a node and watching it vanish and
--    come back for the public role.
-- 3. UPDATE on `nodes`, role `user`: `deleted_at`, `deleted_by`, `deleted_root`
--    added to the columns, so a client can bin one. NOT `path`: the trigger owns
--    it and nothing else should ever write it.
-- 4. Still to do, with the bin app itself:
--      create view deleted_nodes as
--        select * from nodes where deleted_at is not null and id = deleted_root;
--    tracked, select-only, for context owners.

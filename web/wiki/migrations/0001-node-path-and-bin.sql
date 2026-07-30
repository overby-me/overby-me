-- 0001: node paths, and the columns a bin is built on.
--
-- Additive and backward compatible: old.radikal.wiki names none of these
-- columns, so it cannot see them, and no behaviour changes until a client
-- starts writing `deleted_at`.
--
-- Apply through the Hasura console's SQL runner (Data -> SQL) or psql, then
-- refresh graphql/schema.graphql by introspection so cynic can see the new
-- columns. The Hasura metadata changes that go with it are listed at the end;
-- they are NOT SQL and have to be made in the console or through /v1/metadata.
--
-- Everything here is idempotent, so a partial run can be repeated.

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

-- ── path maintenance ─────────────────────────────────────────────────────
-- Deliberately never raises: a write that fails because a denormalised cache
-- could not be computed would be a far worse bug than a null path.
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
  after update of path on nodes
  for each row
  -- Depth guard: the cascade above updates `path` on each descendant, which
  -- would otherwise re-enter this trigger once per row and re-do work already
  -- done by the single statement.
  when (pg_trigger_depth() < 2)
  execute function nodes_cascade_path();

-- ── backfill ─────────────────────────────────────────────────────────────
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

-- ── indexes ──────────────────────────────────────────────────────────────
-- Prefix matching for the subtree operations (LIKE 'x/%').
create index if not exists nodes_path_prefix_idx on nodes (path text_pattern_ops);

-- One live node per path. Partial, so a binned node does not block a new one
-- from taking its place, and does not block its own restore either.
create unique index if not exists nodes_path_live_idx
  on nodes (path) where deleted_at is null and path is not null;

-- Same reasoning for the key within a parent. NOTE: this one can fail on
-- existing data. Check first, and clean up what it reports:
--
--   select parent_id, key, count(*) from nodes
--    where deleted_at is null group by 1, 2 having count(*) > 1;
--
create unique index if not exists nodes_parent_key_live_idx
  on nodes (parent_id, key) where deleted_at is null;

commit;

-- ── consistency check (run any time; zero rows means correct) ─────────────
-- with recursive t as (
--   select id, ''::text as p from nodes where parent_id is null
--   union all
--   select n.id, case when t.p = '' then n.key else t.p || '/' || n.key end
--     from nodes n join t on n.parent_id = t.id
-- )
-- select n.id, n.path as stored, t.p as computed
--   from nodes n join t using (id)
--  where n.path is distinct from t.p;

-- ── Hasura metadata, not SQL ──────────────────────────────────────────────
-- 1. Reload the schema so the new columns appear.
-- 2. SELECT permission on `nodes`, every role: add `deleted_at: {_is_null: true}`
--    to the filter, and add `path` to the allowed columns. This is what makes a
--    binned node invisible, in the new app AND in old.radikal.wiki, with no
--    client change.
-- 3. UPDATE permission on `nodes`: add `deleted_at`, `deleted_by`, `deleted_root`
--    to the allowed columns for whoever may already update a node, so a client
--    can bin one. Do NOT add `path`: the trigger owns it and nothing else should
--    ever write it.
-- 4. The bin view comes with the app that reads it (a later migration):
--      create view deleted_nodes as
--        select * from nodes where deleted_at is not null and id = deleted_root;
--    tracked, select-only, for context owners.

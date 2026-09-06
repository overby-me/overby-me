-- 0026: context_touch, one row per context that moves when anything in it does.
--
-- APPLIED to production on 2026-09-06 (SQL below via run_sql; the metadata
-- change quoted at the end via the metadata API, 0022-style).
--
-- WHY. Every live view held its own change-token subscription (a count +
-- max(updated_at) aggregate filtered to its rows), and Hasura re-runs each
-- distinct (query, variables, session) cohort every second. Cohort count
-- therefore scaled with users x open views, and each cohort's poll evaluated
-- the whole nodes RLS tree per candidate row. That arithmetic is what
-- saturated the instance during HB1 (see 0025 and the incident notes).
--
-- THE SIGNAL. This table carries no content, only "something in this context
-- changed": a per-context sequence number bumped by trigger on every insert,
-- update or delete of a node in that context. Subscribers watch the ONE row of
-- their context; when all components of a client subscribe with an identical
-- query and identical variables, Hasura folds every view of a user, and every
-- tab of that user, into a single cohort row whose poll reads one row and
-- checks one RLS probe (may you see the context node). Update latency is
-- unchanged: the same 1 s poll cadence, a far cheaper poll.
--
-- Deliberate semantics, so nobody "fixes" these later:
--
--   * No FK to nodes: the trigger must never be the reason a write fails, and
--     a touch row surviving a hard-deleted context is invisible (its `node`
--     relationship matches nothing) and harmless.
--   * A context root's own row belongs to its PARENT's context as far as
--     watchers are concerned (a folder listing the root wants its renames), so
--     the trigger bumps the parent context as well for root rows, and skips
--     the self-bump when the root itself is hard-deleted.
--   * Soft delete is an UPDATE here (0024 knows), so it bumps like any edit.
--   * A move rewrites path/ancestors on every descendant and each rewrite
--     re-bumps the same row inside one transaction. Bounded and idempotent;
--     row-level stays simpler than transition tables at this size.
--   * Visibility mirrors the nodes select rule THROUGH the context node: you
--     see the signal iff you may see the context root. A user whose only
--     access is node-scoped membership deeper in the tree gets no signal and
--     degrades to the focus-refresh path the app already has. If the nodes
--     select permission changes, mirror it in the context_touch permission
--     (same drift rule as 0022/0024).

create table if not exists context_touch (
    context_id uuid primary key,
    seq        bigint      not null default 1,
    touched_at timestamptz not null default now()
);

create or replace function bump_context_touch() returns trigger
language plpgsql as $fn$
declare
    ctx        uuid;
    row_id     uuid;
    parent     uuid;
    parent_ctx uuid;
begin
    if tg_op = 'DELETE' then
        ctx := old.context_id; row_id := old.id; parent := old.parent_id;
    else
        ctx := new.context_id; row_id := new.id; parent := new.parent_id;
    end if;
    if ctx is null then
        return null;
    end if;
    if not (tg_op = 'DELETE' and ctx = row_id) then
        insert into context_touch as t (context_id) values (ctx)
        on conflict (context_id)
        do update set seq = t.seq + 1, touched_at = now();
    end if;
    if ctx = row_id and parent is not null then
        select n.context_id into parent_ctx from nodes n where n.id = parent;
        if parent_ctx is not null and parent_ctx <> ctx then
            insert into context_touch as t (context_id) values (parent_ctx)
            on conflict (context_id)
            do update set seq = t.seq + 1, touched_at = now();
        end if;
    end if;
    return null;
end
$fn$;

create trigger nodes_bump_context_touch
    after insert or update or delete on nodes
    for each row execute function bump_context_touch();

-- Seed a row per context that exists today (only for context roots that are
-- still real rows; orphaned context_ids would make invisible rows nobody needs).
insert into context_touch (context_id)
select distinct n.context_id
from nodes n
join nodes c on c.id = n.context_id
on conflict do nothing;

-- To undo:
--   drop trigger if exists nodes_bump_context_touch on nodes;
--   drop function if exists bump_context_touch();
--   drop table if exists context_touch;
--   (and pg_untrack_table context_touch via the metadata API)

-- THE METADATA, applied as one `bulk` via /v1/metadata: track the table with
-- the house naming, a manual object relationship to the context node, and a
-- select permission per role that mirrors the nodes select filter THROUGH that
-- relationship. Quoted for the record; the verification below checks the live
-- copy.
--
-- pg_track_table  {schema public, name context_touch}
--   custom_root_fields: select contextTouch, select_by_pk contextTouchByPk,
--                       select_aggregate contextTouchAggregate
--   column_config: context_id -> contextId, touched_at -> touchedAt
-- pg_create_object_relationship  name node,
--   manual_configuration column_mapping {context_id -> id} -> public.nodes
-- pg_create_select_permission  role user, columns [context_id, seq, touched_at],
--   filter {"node": <the nodes select filter for role user, verbatim>}
-- pg_create_select_permission  role public, columns [context_id, seq, touched_at],
--   filter {"node": <the nodes select filter for role public, verbatim>}
--
-- Verification: the trigger fires and the metadata carries the table.
--
--   begin;
--   update nodes set index = index where id =
--       (select id from nodes where context_id = id limit 1);
--   select seq from context_touch
--       where context_id = (select id from nodes where context_id = id limit 1);
--   rollback;
--
--   select count(*) from hdb_catalog.hdb_metadata
--   where metadata::jsonb -> 'sources' -> 0 -> 'tables' @>
--       '[{"table": {"schema": "public", "name": "context_touch"}}]'::jsonb;

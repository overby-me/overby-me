-- 0007: rate limiting in the permission system, and the pixel canvas that uses it.
--
-- APPLIED to production on 2026-07-31.
--
-- The permissions table answers "who may create this?" and could not answer "how
-- often?". Every feature that needed an answer had to invent one. This adds the
-- general mechanism (docs/rate-limiting.md) and the first two mimes that use it.

-- 1. The mechanism -----------------------------------------------------------

-- The shortest time allowed between two of these, by the same person, in this
-- context. Null means no limit, which is every existing row.
--
-- `interval` rather than an integer of seconds or milliseconds: Postgres has a
-- duration type, and measured on this database comparing intervals is FASTER
-- than converting to milliseconds (0.78 us against 1.25 us per comparison), both
-- of them noise beside a 170 us node insert.
alter table permissions add column if not exists rate_limit interval;

-- Almost no permission row will ever carry a limit, and the trigger runs on every
-- node insert in the app. This partial index holds ONLY the limited rows, so the
-- overwhelmingly common answer ("nothing is limited here") is a miss in a nearly
-- empty index rather than two lookups in busy ones. Measured: without this the
-- trigger doubled an ordinary insert, 0.17 ms to 0.33 ms.
create index if not exists permissions_rate_limited
    on permissions (context_id, mime_id) where rate_limit is not null;

-- The trigger's lookup of "when did this person last do this here". Without it
-- the check scans; with it, it is one descending index read.
create index if not exists nodes_owner_mime_context_updated
    on nodes (owner_id, mime_id, context_id, updated_at desc)
    where deleted_at is null;

-- Enforce the limit that applies to the ACTOR's role.
--
-- The actor is `new.owner_id`, and it can be trusted: Hasura's insert permission
-- presets owner_id from the JWT (`"owner_id": "x-hasura-User-Id"`), so a client
-- cannot write a node as somebody else to dodge its own cooldown. The role comes
-- from the members table, the same way the rest of the app decides it, because a
-- trigger has no access to Hasura's session variables.
--
-- The refusal carries `retry_after_ms` so the app can say WHEN rather than
-- "something went wrong" — see errors.rs.
create or replace function enforce_rate_limit() returns trigger
language plpgsql as $$
declare
    actor_role text;
    lim interval;
    last_at timestamptz;
    wait_ms bigint;
begin
    if new.owner_id is null or new.mime_id is null or new.context_id is null then
        return new;
    end if;

    -- Cheapest possible exit first: is anything limited for this mime here?
    if not exists (select 1 from permissions p
                    where p.context_id = new.context_id
                      and p.mime_id = new.mime_id
                      and p.rate_limit is not null
                      and p.active) then
        return new;
    end if;

    select case
               when exists (select 1 from members m
                             where m.parent_id = new.context_id
                               and m.node_id = new.owner_id
                               and m.owner)
               then 'owner' else 'member'
           end
      into actor_role;

    select p.rate_limit into lim
      from permissions p
     where p.context_id = new.context_id
       and p.mime_id = new.mime_id
       and p.role = actor_role
       and p.active
       and p.rate_limit is not null
     limit 1;

    if lim is null then
        return new;
    end if;

    -- The actor's last action on this mime here. `updated_at` rather than
    -- `created_at`, so repainting an existing row counts as an action; the row
    -- being written is excluded, or an update would always find itself.
    select max(n.updated_at) into last_at
      from nodes n
     where n.owner_id = new.owner_id
       and n.mime_id = new.mime_id
       and n.context_id = new.context_id
       and n.deleted_at is null
       and n.id <> new.id;

    -- On a repaint, the row's OWN previous timestamp is the actor's last action.
    -- Excluding it above (or an update would always find itself) would otherwise
    -- let somebody repaint one pixel as fast as they liked.
    if tg_op = 'UPDATE' then
        last_at := greatest(last_at, old.updated_at);
    end if;

    if last_at is not null and (now() - last_at) < lim then
        wait_ms := ceil(extract(epoch from (lim - (now() - last_at))) * 1000)::bigint;
        raise exception 'rate limited: retry_after_ms=%', wait_ms
            using errcode = 'P0001';
    end if;

    return new;
end $$;

drop trigger if exists nodes_rate_limit on nodes;
create trigger nodes_rate_limit
    before insert or update on nodes
    for each row execute function enforce_rate_limit();

-- 2. The pixel canvas --------------------------------------------------------

-- `pixel/canvas` is ordinary content: it appears in folders, the drawer and
-- search like a document, so a context can hold as many canvases as it likes.
-- `pixel/pixel` is hidden, exactly as `vote/vote` is, so four thousand of them
-- never show up in a listing, a search result or the sort order.
insert into mimes (id, "unique", hidden, context, icon, traits)
values ('pixel/canvas', false, false, false, 'grid_on', '[]'::jsonb)
on conflict (id) do nothing;

insert into mimes (id, "unique", hidden, context, icon, traits)
values ('pixel/pixel', false, true, false, 'grid_on', '[]'::jsonb)
on conflict (id) do nothing;

-- Permissions are per context and are seeded by the app when a canvas is created
-- (graphql::create_canvas, mirroring create_speaker_list), so nothing is granted
-- anywhere until somebody actually makes one.

-- To undo:
--   drop trigger if exists nodes_rate_limit on nodes;
--   drop function if exists enforce_rate_limit();
--   drop index if exists nodes_owner_mime_context_updated;
--   alter table permissions drop column if exists rate_limit;
--   delete from mimes where id in ('pixel/canvas','pixel/pixel');

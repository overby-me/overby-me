-- 0010: a rate limit is about making content, not about removing or moving it.
--
-- Reported from production on 2026-08-01: deleting a folder failed with
-- `rate limited: retry_after_ms=54977`. The statement was the ordinary subtree
-- soft delete (`set deleted_at ... where path like 'x/y/bla/%'`).
--
-- 0007 put the trigger on `before insert or update`, and every one of these is
-- an UPDATE of a `pixel/pixel` row:
--
--   * soft-deleting a folder, which rewrites every descendant. A canvas holds
--     up to a thousand cells, so the FIRST cell in the statement sets the
--     others' clock and the delete cannot get past the second one. The whole
--     delete then rolls back, so a canvas anywhere under a folder made that
--     folder undeletable until the cooldown passed - and with more cells than
--     seconds of cooldown, never.
--   * restoring one from the bin, the same statement in reverse.
--   * renaming or moving an ancestor, which rewrites `path` and `ancestors` on
--     every descendant.
--
-- None of those are a person painting a pixel, which is the only thing the
-- limit exists to slow down. The limit now applies to a row being CREATED, or
-- to an update that changes what the row says.
--
-- APPLIED to production on 2026-08-01.

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

    -- Removing content, or putting it back, is not the action being limited. A
    -- soft delete is an UPDATE, so without this a cooldown meant for one pixel
    -- blocked the delete of everything above it.
    if new.deleted_at is not null
       or (tg_op = 'UPDATE' and old.deleted_at is not null) then
        return new;
    end if;

    -- Nor is bookkeeping. Moving or renaming an ancestor rewrites `path` and
    -- `ancestors` on every descendant, and a row whose stored content is
    -- untouched is not its owner acting: only a change to what the row SAYS
    -- counts. (`is not distinct from` so a NULL either side compares equal.)
    if tg_op = 'UPDATE'
       and new.data is not distinct from old.data
       and new.name is not distinct from old.name
       and new.key is not distinct from old.key
       and new.file_id is not distinct from old.file_id
       and new.mime_id is not distinct from old.mime_id then
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

-- The trigger itself is unchanged (0007 created it); only the function body is
-- replaced, so there is nothing to re-attach.
--
-- To undo: re-run the function definition from
-- migrations/0007-rate-limit-and-pixels.sql.

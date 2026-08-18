-- 0014: a locked canvas takes no more paint.
--
-- A canvas already had an open state - `nodes.mutable` on the canvas row, drawn
-- as a "Closed" chip and checked before the board offers itself. It was the
-- client asking nicely: nothing stopped a placement that did not come from the
-- board, because the insert rule for `nodes` looks at whether the person may
-- insert this mime in this context and never at the canvas they are painting
-- on. Closing a board hid the brush; it did not take it away.
--
-- The cooldown next to it is enforced by a trigger (0007) rather than by the
-- client for exactly this reason, so the lock is enforced the same way.
--
-- Two things this must NOT break, both found by thinking about what else writes
-- to a pixel row:
--
--   * Deleting a canvas soft-deletes its cells, which is an UPDATE of every one
--     of them. A lock that refused those would make a locked canvas impossible
--     to throw away.
--   * The same goes for anything that rewrites a row without repainting it - a
--     path or ancestors rewrite, a restore from the bin.
--
-- So the rule is about PLACEMENTS, not about writes: an insert is refused, and
-- an update is refused only when it changes the colour or the painter. Anything
-- else passes.

create or replace function enforce_canvas_lock() returns trigger
language plpgsql as $$
begin
    if new.mime_id is distinct from 'canvas/pixel' then
        return new;
    end if;

    -- Housekeeping on an existing cell, not a placement: let it through.
    if tg_op = 'UPDATE'
       and new.data is not distinct from old.data
       and new.owner_id is not distinct from old.owner_id then
        return new;
    end if;

    if exists (select 1 from nodes c
                where c.id = new.parent_id
                  and c.mime_id = 'canvas/canvas'
                  and not c.mutable
                  and c.deleted_at is null) then
        raise exception 'canvas locked' using errcode = 'P0001';
    end if;

    return new;
end $$;

drop trigger if exists nodes_canvas_lock on nodes;
create trigger nodes_canvas_lock
    before insert or update on nodes
    for each row execute function enforce_canvas_lock();

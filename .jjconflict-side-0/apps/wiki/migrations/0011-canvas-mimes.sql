-- 0011: the app is called canvas, so its mimes are too.
--
--   pixel/canvas -> canvas/canvas
--   pixel/pixel  -> canvas/pixel
--
-- 0007 named these after the thing they draw; the app is named after the thing
-- you draw ON, and it is the app's name that people read (the rail, the
-- breadcrumbs, the add dialog). Doing it now because the canvas has barely been
-- used: six nodes site-wide, all in one test event.
--
-- The `relations.name = 'pixel'` anchor, which records the canvas a context is
-- showing, is renamed with them. Half a rename is worse than none: the next
-- person to read `relations` would find a `pixel` pointing at a `canvas`.
--
-- Two triggers are held off for the rename:
--
--   * `nodes_rate_limit`, because a mime change is a change to what the row says
--     and would otherwise be checked against the cooldown, which would either
--     fail the migration or - worse - pass it while consuming somebody's turn.
--   * `set_public_nodes_updated_at`, so the cells keep the timestamps of when
--     they were actually painted. `updated_at` IS the rate limiter's clock, and
--     renaming a mime is not painting.
--
-- The path/ancestors triggers do not fire: they are declared UPDATE OF
-- parent_id/key, and neither moves here.
--
-- APPLIED to production on 2026-08-01.

begin;

alter table nodes disable trigger nodes_rate_limit;
alter table nodes disable trigger set_public_nodes_updated_at;

-- The new mimes first, carrying every property of the old ones, so nothing
-- points at a mime row that does not exist yet.
insert into mimes (id, "unique", hidden, context, icon, traits)
select replace(id, 'pixel/', 'canvas/'), "unique", hidden, context, icon, traits
  from mimes
 where id in ('pixel/canvas', 'pixel/pixel')
on conflict (id) do nothing;

update nodes set mime_id = replace(mime_id, 'pixel/', 'canvas/')
 where mime_id in ('pixel/canvas', 'pixel/pixel');

update permissions set mime_id = replace(mime_id, 'pixel/', 'canvas/')
 where mime_id in ('pixel/canvas', 'pixel/pixel');

-- `parents` is a text[] of mime ids (a cell's parent is a canvas), so the rename
-- has to reach inside it too. `array_replace` keeps the order, which a
-- round trip through unnest/array_agg would not promise.
update permissions set parents = array_replace(parents, 'pixel/canvas', 'canvas/canvas')
 where 'pixel/canvas' = any(parents);
update permissions set parents = array_replace(parents, 'pixel/pixel', 'canvas/pixel')
 where 'pixel/pixel' = any(parents);

update relations set name = 'canvas' where name = 'pixel';

delete from mimes where id in ('pixel/canvas', 'pixel/pixel');

alter table nodes enable trigger set_public_nodes_updated_at;
alter table nodes enable trigger nodes_rate_limit;

commit;

-- To undo: the same statements with the replacement reversed
-- ('canvas/' -> 'pixel/'), and `relations.name = 'canvas'` back to 'pixel'.

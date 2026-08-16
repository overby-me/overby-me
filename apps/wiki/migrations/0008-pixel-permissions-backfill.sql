-- 0008: give every existing context the right to hold a pixel canvas.
--
-- APPLIED to production on 2026-07-31.
--
-- 0007 left this to the app: creating a canvas seeds the permissions it needs,
-- the way a speaker list does. That works, but it means the "Pixel canvas" entry
-- only appears for a context owner, and only because the add dialog special-cases
-- it. Granting the rows up front makes a canvas an ordinary offer of the
-- permission system in every context that already exists, which is what the
-- system is for.
--
-- HASURA METADATA, not SQL, and required before any of this is usable from the
-- app: a new column is invisible to GraphQL until the metadata is reloaded, and
-- it cannot be written by a non-admin until it is listed in the role's column
-- permissions. Without it the client's seeding mutation answers "field
-- 'rate_limit' not found in type: 'permissions_insert_input'". Applied via
-- /v1/metadata: reload_metadata, then drop+create of the permissions table's
-- insert (user), select (user, public) and update (user) permissions with
-- `rate_limit` appended to each column list.
--
-- Idempotent: `where not exists` on every insert, so running it twice grants
-- nothing twice, and a context that already has a canvas is left alone.

-- Owners may create a canvas, wherever content lives.
insert into permissions (context_id, node_id, mime_id, role, parents, active,
                         "insert", "select", "update", "delete")
select p.context_id, p.context_id, 'pixel/canvas', 'owner',
       '{wiki/event,wiki/group,wiki/folder}', true, true, true, true, true
  from (select distinct context_id from permissions where context_id is not null) p
 where not exists (select 1 from permissions x
                    where x.context_id = p.context_id
                      and x.mime_id = 'pixel/canvas'
                      and x.role = 'owner');

-- Everybody may paint on one, one cell at a time.
--
-- BOTH roles, with the same limit. The trigger looks up the row matching the
-- actor's role, so an owner with no owner row finds no limit at all and paints
-- without a cooldown — which is not what "one per person" means. Verified: an
-- owner painted twice in a row until the owner row existed.
--
-- A minute is the default a canvas is created with. It is a permission row, so a
-- chair can change it per context without a deploy, and a canvas that wants a
-- different pace can have one.
insert into permissions (context_id, node_id, mime_id, role, parents, active,
                         "insert", "select", "update", "delete", rate_limit)
select p.context_id, p.context_id, 'pixel/pixel', r.role,
       '{pixel/canvas}', true, true, true, true, false, interval '60 seconds'
  from (select distinct context_id from permissions where context_id is not null) p
 cross join (values ('member'), ('owner')) as r(role)
 where not exists (select 1 from permissions x
                    where x.context_id = p.context_id
                      and x.mime_id = 'pixel/pixel'
                      and x.role = r.role);

-- To undo:
--   delete from permissions where mime_id in ('pixel/canvas', 'pixel/pixel');

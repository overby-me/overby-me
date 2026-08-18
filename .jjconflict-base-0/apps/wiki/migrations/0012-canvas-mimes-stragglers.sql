-- 0012: the pixel/* rows a stale tab wrote after 0011 renamed the mimes.
--
-- 0011 renamed the canvas mimes in the database, but a browser tab open from
-- before the matching deploy went on asking for the old ones. It created a
-- canvas and a cell under `pixel/canvas` / `pixel/pixel`, and - because
-- `create_canvas` seeds its permissions when it does not find its mime among
-- the ones it may insert - it re-created the two permission rows as well.
--
-- Nothing here is a design change; it is the wake of a rename, and it is
-- written to be idempotent so it can be re-run if another tab surfaces late.
--
-- The nodes are renamed onto the new mimes. The permission rows are DELETED
-- rather than renamed: 0011 already produced the canvas/* equivalents, and the
-- stale pair duplicates them exactly (same context, node, role, parents and
-- grants), so renaming them would leave the context with two identical rules.
--
-- APPLIED to production on 2026-08-01.

begin;

alter table nodes disable trigger nodes_rate_limit;
alter table nodes disable trigger set_public_nodes_updated_at;

update nodes set mime_id = replace(mime_id, 'pixel/', 'canvas/')
 where mime_id in ('pixel/canvas', 'pixel/pixel');

-- Drop a stale rule only where the renamed one it duplicates already exists;
-- anything without a counterpart is renamed instead, so no context silently
-- loses a permission it still needs.
delete from permissions p
 where p.mime_id in ('pixel/canvas', 'pixel/pixel')
   and exists (
       select 1 from permissions q
        where q.context_id = p.context_id
          and q.node_id is not distinct from p.node_id
          and q.role = p.role
          and q.mime_id = replace(p.mime_id, 'pixel/', 'canvas/'));

update permissions set mime_id = replace(mime_id, 'pixel/', 'canvas/')
 where mime_id in ('pixel/canvas', 'pixel/pixel');

update permissions set parents = array_replace(parents, 'pixel/canvas', 'canvas/canvas')
 where 'pixel/canvas' = any(parents);
update permissions set parents = array_replace(parents, 'pixel/pixel', 'canvas/pixel')
 where 'pixel/pixel' = any(parents);

update relations set name = 'canvas' where name = 'pixel';

alter table nodes enable trigger set_public_nodes_updated_at;
alter table nodes enable trigger nodes_rate_limit;

commit;

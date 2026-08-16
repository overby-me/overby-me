-- 0009: a canvas is reached through its app, not through the folder listing.
--
-- APPLIED to production on 2026-07-31.
--
-- 0007 made `pixel/canvas` visible content, so a canvas sat in folders among the
-- documents and had to be created from the add dialog. It belongs with the
-- speaker list instead: a hidden mime with an app of its own on the rail
-- (`?app=pixel`), which lists the context's canvases and creates them. Hidden
-- keeps it out of folder listings, search and the sort order, exactly as
-- `speak/list` and `vote/poll` are kept out.
--
-- A canvas still has a path and still resolves: hidden governs listings, not
-- navigation, so opening one from the app works and the link can be shared.

update mimes set hidden = true where id = 'pixel/canvas';

-- To undo:
--   update mimes set hidden = false where id = 'pixel/canvas';

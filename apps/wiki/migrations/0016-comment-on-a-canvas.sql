-- 0016: a canvas can be talked about.
--
-- The canvas page now carries the comment section every other thing worth
-- discussing has. That is a client change, but it would have been a client
-- change that did not work: a comment is a node whose permission row lists the
-- mimes it may hang off, and `canvas/canvas` was not among them, so the box
-- would have been there and the send would have been refused.
--
-- Appended to what is already listed rather than replacing it, and only where
-- it is missing, so a context that has been given a narrower or wider list than
-- the usual one keeps it.
--
-- Every context, not only the ones that have a canvas today. The row says what a
-- comment MAY hang off, and a context with no canvas is unaffected by the extra
-- entry; scoping it to contexts that happen to have one now would just mean the
-- next context to make a canvas is quietly broken.

update permissions
   set parents = array_append(parents, 'canvas/canvas')
 where mime_id in ('vote/comment', 'vote/reaction')
   and not ('canvas/canvas' = any(parents));

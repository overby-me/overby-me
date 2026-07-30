# Notes on the open issues

Written 2026-07-31, after working through the small ones. Three of the issues
turned out to need a decision rather than an implementation, and one turned out
to be already done. This is what was found, so the decisions can be made from
evidence rather than from memory.

## Already delivered: #11 (random ballot order), #7 in part

**#11 is implemented** in the app deployed on 2026-07-30 and was filed against
the React app that `radikal.wiki` served until that evening. `ballot_order()`
shuffles the options with Blank pinned last, memoised per poll and option count
so it cannot reshuffle under a voter mid-ballot. Both branches of the ballot —
single-choice radios and multi-choice checkboxes — iterate the shuffled order
while selecting by the ORIGINAL index, so the stored vote is unaffected by what
the screen showed. Covered by `ballot_order_keeps_blank_last_and_is_a_permutation`.

**#7** was half delivered for the same reason: documents autolinked, comments
and plain text nodes did not. They do now.

## Blocked on a decision: images on a comment

The client half is straightforward and was written; it is not committed, because
the images it produces would be **readable by nobody**. What the permission model
actually says, verified against production:

- `storage.files` is readable by role `user` under two conditions: the file was
  uploaded by them (`uploaded_by_user_id`), or it is referenced by a NODE they
  may read, through a manual relationship mapping `files.id -> nodes.file_id`.
- `uploaded_by_user_id` is **null on all 1023 files**. That branch has never
  matched anyone. The only path that works is the node reference.
- The reference is the `nodes.file_id` COLUMN, not anything inside `data`. Every
  node in the wiki that carries a file sets it — 653 of them, with no exceptions.
- `file_id` is neither selectable nor insertable by `public` or `user`. A client
  cannot write it, and no backend route does either.

Verified end to end: a member of the context can read a file behind a node's
`file_id`, someone outside it gets nothing, and an anonymous reader can read it
when the node is public. Exactly the behaviour wanted — but only reachable
through a column the client may not write.

So an image attached to a comment cannot be made readable today. Three ways out,
in the order I would pick them:

1. **A child `wiki/file` node per attachment.** This is how the wiki already
   models a file, so the permissions, the bin, restore and orphan cleanup all
   work with no server change. The comment renders its file children. More code
   than the alternatives, and comment queries would have to fetch children.
2. **A backend attach route.** The backend holds admin rights; it could create
   the file node (and set `file_id`) after checking the caller may comment on
   that parent. Keeps one image per comment simple, but adds a route that writes
   nodes, which the backend does not do today.
3. **Widen the client's insert permission to `file_id`.** Smallest change,
   largest blast radius: any client could then point a node at any file id,
   including one it may not read, and use a readable node as a lever to expose
   it. I would not do this.

I stopped rather than pick, because it is a data-shape decision that outlives
the feature.

## Needs design: #10, node revision table

Not started deliberately. The schema is the easy part; the questions are:

- **What is a revision?** Every autosave (the editor debounces at a few seconds,
  so a long edit is hundreds), every explicit save, or every SUBMIT? The last is
  the one with meaning in this wiki: a policy becomes immutable when submitted,
  and the interesting question is what changed between two submitted versions.
- **What is kept?** A full `data` snapshot per revision is simple and, at the
  size of these documents, cheap — thousands of nodes at a few KB each. A diff
  is smaller and much harder to render.
- **Who sees them?** Probably anyone who can read the node. A revision can
  contain text an author later removed on purpose, which is an argument for
  owner-only, and against keeping them for comments at all.
- **How long?** Forever is a decision, not a default.

My suggestion: `node_revisions (id, node_id, data, name, created_at, author_id)`,
written by a trigger on `nodes` when a SUBMITTED node's `data` or `name` changes,
content mimes only, readable by whoever can read the node. That is one migration
and a history view, and it deliberately does not record drafts.

## Needs design: #4, open contexts

"Contexts where anyone can join." The permission model already has the pieces —
`permissions` rows keyed by context, mime, role and parent mime, plus the
`members` table with `accepted` — so an open context is mostly a policy question:

- Does joining make you a `member` immediately, or a member with `accepted =
  false` that an owner confirms? The second is the current invitation shape and
  needs no new state.
- What can a self-joined member do? The `permissions` rows already say what a
  member may insert, update and delete per mime; an open context probably wants
  a NARROWER set than an invited one (comment and vote, not create).
- Can they leave, and does leaving remove their votes? It must not.
- Is the context discoverable before joining? Today a context you are not in is
  invisible unless it is public.

This one is worth a conversation before code: it is the first feature that lets
someone into a context without an owner acting, which is a security boundary
rather than a feature flag. Everything else in this file is reversible; this is
not.

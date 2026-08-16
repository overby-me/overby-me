# Notes on the open issues

Written 2026-07-31, after working through the small ones. Two of them turned out to need a
decision rather than an implementation, and one turned out to be already done. This is what was found, so the decisions can be made from
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

## Done: images on a comment

Straightforward in the end, once the mechanism was read rather than inferred.
The first pass here concluded it was blocked, on the grounds that a file is only
readable through the `nodes.file_id` column and no client may write it. Both
halves of that are true and the conclusion was still wrong: **`file_id` is a
GENERATED column**, derived from `data->>'image'` (or `data->>'fileId'`) and
validated as a uuid. The client never writes it. It writes `data.image`, exactly
as a candidate's photo and a document's cover image already do, and the column —
and with it the storage permission — follows.

So a comment carries one image, stored at `data.image`. One, because the
generated column holds one uuid; a second would be invisible, which is worse than
not offering it.

Verified against production with the shape the app now inserts: a member posted a
comment with `data.image`, the `file_id` column generated itself, and a DIFFERENT
member of that context could read the file row. The probe was removed afterwards.

What is worth knowing about the permission, since it is not obvious:

- `storage.files` is readable when the file was uploaded by the reader
  (`uploaded_by_user_id`) or when a NODE the reader may read points at it.
- `uploaded_by_user_id` is **null on all 1023 files** in this deployment. That
  branch has never matched anyone, so the node reference is the only path — which
  is why storing an id anywhere other than where the generated column reads it
  produces an image nobody can see, not even its author.

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

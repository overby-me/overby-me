# Cutover rehearsal, on a dump of production

`docs/cutover-runbook.md` was paper until 2026-09-20, when the owner asked for
it to be run. This is what was run, what it found, and what it counted. Counts
and kinds only: the dump holds every member's address, was kept in a private
scratch directory, and was removed afterwards. Nothing of it is in the
repository.

Run it again, from a fresh dump in `<dir>`, with
`nu scripts/rehearse-cutover.nu <dir>`, and then
`nu scripts/rehearse-cutover-browser.nu <dir>` for the last step.

## What was run

1. **The dump**, read-only, a page of 500 rows at a time with a pause between,
   on a Sunday morning: 8510 nodes, 21730 member rows, 336 users, 6 rows that
   open a context to everyone. Each table matched its own count. The files
   after it, one at a time: 1166 files, 806 MB.
2. **Extract.** 336 users (314 to be recognized by address), 41 contexts (2
   public), 3613 documents, 18976 members, 168 comments, 16 reactions, 29
   polls, 3 canvases, 7 reports.
3. **Load** (`appview import`), into a datastore that did not exist: under 3
   seconds.
4. **Files** (`appview import-files`): 654 copied, none refused. 512 files in
   storage are pointed at by nothing that is carried (354 MB of the 806): 27 by
   rows that are left behind, the rest by nothing in the interim at all.
5. **The gates** (`appview verify`): all eight green. 23125 rows known by the
   id they had, 16498 waiting seats of 1940 addresses with none twice in a
   context, 3555 live paths that agree with the tree, 2348 authors, 654 of 654
   files whole by size and by hash.
6. **Everyone returns.** Each of the 314 accounts that can be recognized by
   address was signed in under a DID of its own, as its provider would have
   confirmed the address. None failed; 4869 seats changed hands; no seat was
   left with an account nobody can sign in to.
7. **Smoke test** over HTTP, 16 of 16: the two public contexts open to nobody
   and a closed one does not; a returned member has the seats of their old
   account (23 contexts); a member list of 822 answers to a member and not to a
   stranger; a page with several authors opens with all of them, at its old
   URL, and not to a stranger; a comment lands and is read back; search finds
   the page by a word of its title, and finds a stranger nothing; a file is
   served whole to a member, and refused to a stranger and to nobody.
8. **The frontend, in a browser** (`scripts/rehearse-cutover-browser.nu`): the
   build on the AppView in headless Firefox, as the returned member with the
   most seats, walking real pages: ten of every kind, the longest bodies first,
   since those are the likeliest to hold a shape no made-up page has. All 91
   drew their title, and the app logged no error.

## What it found

The first load of real data failed, twice. Both were the extractor's, and both
are fixed and tested there.

- **Orphans.** 528 rows hang off a parent row that is gone: 95 subtrees whose
  top was deleted outright before the interim had a bin (mostly 2023 to 2025,
  touched since only by batch fixes). The interim keeps a null `path` for them
  and reaches none of them by any URL. The extractor re-rooted them silently at
  the top of their group, under a path made of their own key, and two of them
  with one key failed the whole load on the path index. They are now left
  behind on purpose, counted by kind (162 policies, 191 amendments, 42 folders,
  42 files, 32 documents, 38 comments, 15 positions, 4 candidates, 2
  reactions), with the 431 author rows on them. Carried, they would have
  brought deleted content back.
- **Deleted accounts.** `members.nodeId` is tied to nothing in the interim, so
  an account can be deleted from under its rows: 3 seats and 1 author chip
  named an account that no longer exists, and failed the load on a foreign key
  that named no row. A seat goes back to waiting for its address (a roster says
  who belongs, account or none); an author is carried by name. The loader now
  refuses an unknown account by name (`UnknownAccount`), where the engine said
  only that a foreign key failed.
- **Decisions filed as gaps.** A context's owner realized as an owner
  membership (36) and a poll open at the dump carried closed (2) were reported
  under `unmapped_source`, which the gate holds to empty, so the gate could
  never have been green. They are decisions, and have a bucket of their own
  (`reshaped`).
- **Two legacy one-off pages** (`conference/conference`, `map/map`), each with
  no data and nothing under it, are left behind as empty shells. One that held
  anything would still be a gap.
- **The gates were prose.** On the host nobody can look into the service's
  private state directory, so a gate that is not a command is a gate nobody
  checks. They are `appview verify` now, and the import unit runs them.
- Pictures inside page bodies point at other sites or are inline, never at the
  interim's storage, so the file copy misses none. Avatars are Gravatar's.

## What it did not cover

- The interim was not frozen, so two polls were open at the dump. On the day
  they are closed first, or come across closed with what they had taken.
- 22 accounts have no verified address and will need a claim link for each
  seat, from an owner.
- A real sign-in. The rehearsal stands in for it with a dev tool that is never
  deployed; one is made apart from it, on a made-up account against a real PDS
  (`scripts/test-real-login.nu`), which cannot cover taking an old account over
  by address, since a PDS on plain http is never believed about one.
- The NixOS host: the unit that runs the load and the gates is covered by the
  VM test, on a made-up wiki.
- The later move into atproto spaces (`atproto-spaces-redesign.md`). It has a
  rehearsal of its own, `nu scripts/rehearse-spaces.nu <dir>`, run on what this
  one leaves in `<dir>/stage`. It was written after this dump was deleted, so
  it has run on a made-up wiki only: which of production's pages and files are
  past what a PDS takes, and whether production rebuilds from its records, is
  what the next rehearsal is there to say.

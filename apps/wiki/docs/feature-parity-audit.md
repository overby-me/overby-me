# Feature parity audit: old React wiki vs Dioxus port

Date: 2026-07-15. Method: seven parallel domain audits comparing
`apps/wiki/src` (old React app) against `apps/wiki-dioxus/src` (this port),
each reporting only functional gaps with `file:line` evidence, followed by
manual verification of the top findings. Prompted by the discovery that the
port's "Add content" dialog had silently dropped the ability to create
motions (`vote/policy`) and elections (`vote/position`) — this audit looked
for more of the same.

## Headline

There is **no second whole-feature disaster**. The app-switcher is complete:
every app/view the old wiki had (folder, editor, speak, vote, member, sort,
screen, map, poll) is reachable in the port, and the port adds several new
ones (follow, admin/console, graph, program, social, public profile). Most
domains are at parity or ahead. The regressions that remain are one genuine
"capability silently dropped" bug (cover images, same class as the
motion/election one) plus a cluster of UX-richness reductions.

The motion/election gap that triggered this audit is already fixed
(`folder.rs:596`, deriving the type menu from `node_insert_mimes`).

> **Status (2026-07-15): all five regressions below (#1-#5) are now fixed** in
> commit "Restore cover images, backdating, amendment previews, reorder and poll
> delete", verified by the browser E2E (115/115). This document is kept as the
> audit record; the per-finding fix locations are noted inline.

## Regressions, by severity

### HIGH

1. **Cover / hero image can no longer be set on a content node.**
   - Old: the editor had a `FileUploader` writing `data.image`
     (`content/Editor.tsx:175`).
   - New: `content.rs:91-105` still *renders* `data.image` as a full-bleed
     hero (`has-hero`), but `editor.rs` only *preserves* it (`editor.rs:310`) —
     there is no field to add or change it. A document/motion/policy can show a
     cover image but nobody can give it one.
   - Same class as the motion/election bug: the upload plumbing exists
     (`nhost::upload_file`, wired into candidate photos and file nodes) but was
     never surfaced in the general editor.
   - Fix: re-add an image uploader to the editor's metadata area.

### MED

2. **Backdating content is gone.** Old: context owners saw a `DatePicker`
   bound to `createdAt` (`content/Editor.tsx:169`) to set an agenda item's or
   minutes' date. New: no date control; the save path omits `created_at`
   (`editor.rs:355`). Fix: owner-only date field in the editor sidebar +
   include `created_at` in the update.

3. **Amendment rows lost their inline preview and author chips.** Old:
   `vote/ChangeList.tsx:42-67` rendered each amendment's full body inline
   (expand/collapse) plus author `MemberChips`. New: `vote/policy.rs:79-93`
   shows a bare name + avatar link; you must open each amendment to read it or
   see who wrote it. Fix: render the amendment body inline (`SlateRenderer`) +
   author chips in the list.

4. **Reorder (sort) is unreachable for candidates / questions / amendments /
   comments.** Old: each vote sub-list carried an owner-only "sort" button
   (`ChangeList.tsx:135`, `QuestionList.tsx:55`, `CommentList.tsx:53`,
   `CandidateList`). New: the generic `SortApp` still exists with native
   drag-and-drop (`sort.rs`) but is only surfaced from the folder sheet
   (`folder.rs:310`), which motion/position pages do not render — so an owner
   can no longer reorder candidates in an election, amendments on a motion,
   etc. They fall back to insertion order. (Confirmed by two independent
   audits.) Fix: add an owner-gated "Reorder" action to the PolicyApp /
   PositionApp section headers that routes to `?app=sort`.

### LOW

5. **Client-side draft-candidacy hiding dropped.** Old: `CandidateList.tsx:22`
   hid other members' still-mutable candidacies (`mutable=false OR owner=me OR
   member=me`). New: `visible_sorted` (`loader.rs`) filters only on the mime
   `hidden` flag, so all candidate rows render. Downgraded to LOW after
   verification: the port's candidate creation is atomic (one dialog inserts a
   complete candidate, `position.rs:442`), so there is no multi-step "draft"
   period to leak, and the actual visibility boundary (non-members cannot see
   candidacies) is Hasura row-level security, not this client filter.

6. **Poll list dropped the owner's per-poll delete button.** Old:
   `PollList.tsx:74`. New: poll-list rows are navigation-only
   (`policy.rs`/`position.rs`). A stray/mistaken poll can no longer be removed
   from the list UI.

7. **Map downgraded from interactive MapLibre to a static OSM iframe.** Old:
   `MapApp.tsx` (MapLibre WebGL). New: `map.rs` (OpenStreetMap iframe over the
   same bbox). A deliberate tradeoff to keep a WebGL lib out of the wasm
   bundle; neither version plots node markers, so impact is cosmetic.

8. **Comment ordering is not owner-sortable.** Old: `CommentList.tsx:32`
   ordered by `index`. New: the threaded `comments.rs` fetches in
   insertion/thread order with no reorder UI. Consistent with the new nested
   model.

9. **Document export format changed `.docx` -> `.odt`.** Lateral swap
   (`export.rs`); the recursive subtree export with embedded images is intact,
   arguably richer.

## Enhancements missing in BOTH versions (not regressions, possible backlog)

- Results export to CSV / print (only the member roster exports CSV).
- Resend a pending invitation.
- Reopen a closed poll / re-voting.
- Diff/highlight amendments against the motion text.
- Insert-image toolbar button in the rich-text editor (both only accept pasted
  `<img>`).

## Intentional drops — do NOT re-flag as bugs

Confirmed against project decisions/memory:

- **Delegation / proxy / weighted voting** — dropped by decision (org does not
  use it).
- **Gender / category-balanced speaker queue** — deferred pending an org policy
  on the balancing rule; the `priority`/category field is unused in both
  versions.
- **Editable permissions UI** — the old `PermApp`/`PermList` was entirely
  commented-out dead code; the port actually *added* a working read-only
  permissions table. No edit UI exists in either (authz is server-enforced).
- **Question delete UI** — removed intentionally in an earlier session.
- **@-mention parsing** — deliberately not built.

## Recommended fix order

1. Cover/hero image uploader in the editor (HIGH — a real dropped capability).
2. "Reorder" action on motion/position section headers (MED — unlocks the
   already-built SortApp for candidates/questions/amendments).
3. Amendment inline preview + author chips (MED — deliberation legibility).
4. Owner backdating date field (MED — needed for minutes/agenda dating).
5. Poll-list delete button (LOW — small owner affordance).

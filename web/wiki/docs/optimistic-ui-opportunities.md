# Optimistic UI Opportunities

## Intro

Optimistic UI means reflecting the user's action in the interface immediately (before the server confirms it), then reconciling against the server result and rolling back on error. It pays off most for frequent, latency-sensitive, user-initiated actions on a visible list or toggle where the change is cheap to represent locally and cheap to undo.

Today, every data-mutating action in wiki-dioxus updates the UI through one of three round-trip mechanisms, none of which is instant:

- `bump_data_version()`: invalidates cached resources so they refetch (a network round-trip before the change shows).
- `crate::subscription::use_live(query, refresh)`: a Hasura WebSocket subscription that bumps a `refresh` signal when server data changes (round-trip plus WS latency).
- a local `refresh`/`rev` signal plus `use_data_resource!` that re-runs the query.

So after casting a vote, joining a speaker queue, reordering, inviting a member, adding a candidate, or projecting an item, the UI sits unchanged until the refetch/subscription lands (typically 200ms to 2s).

The one existing optimistic pattern is comments (`src/components/comments.rs`): a `PendingComment { key, ... }` is pushed to a local pending `Vec` and rendered immediately as a muted `.comment-pending` row (`comments.rs:26-49`). `reconcile_pending()` drops the pending row once the refetch returns a real comment with the same key (no duplicate, no flicker), and errors surface via `crate::snackbar::show_snackbar`. New work should reuse this reconcile-by-key template.

Count after dedup and verifier adjustments: 6 High, 15 Medium, 3 Low.

## The reusable patterns

Three recipes cover every opportunity below. Build the shared reconcile-by-key helper once (generalize `comments.rs:40-49` off `PendingComment` so any `{key, ...}` row type can reconcile against a `HashSet<String>` of fetched keys).

### (a) Reconcile-by-key for list INSERTS (the PendingComment template)

For joins, invites, questions, candidates, pastes, and child adds. Generate a unique client-side key (timestamp- or name-based, matching how the mutation keys the node). Push a `Pending{...}` row to a local pending `Vec` signal and render it immediately in a muted "sending" style. Spawn the mutation; on success bump the refresh signal so the refetch lands and `reconcile_pending()` drops the pending row once a real row with the same key appears. On error: remove the pending entry, restore any input text, and `show_snackbar` the error. Rollback is race-safe because reconciliation is purely by key.

### (b) Local override signal for TOGGLES and reorders (flip immediately, clear on reconcile/error)

For lock/unlock, close-poll, project/stop-projecting, focus-section, owner/active/hidden toggles, timer, and speaker reorder. Hold a small override signal (`Signal<Option<bool>>`, `Signal<Option<String>>`, or a `Map<Id, state>`) that shadows the server value. On click, flip the override immediately so the icon/row/highlight reacts at once, then spawn the mutation. On success, let the refetch land and clear the override once the server value matches. On error, clear the override (snapping back to the server value) and `show_snackbar`. No list reconciliation is needed, just a shadow of one field.

### (c) Immediate local REMOVAL for deletes (hide the row, restore on error)

For deleting a comment subtree, removing a speaker, advancing "next speaker", and accepting/declining an invite (the row leaves the list either way). Keep a `deleted_ids: Signal<HashSet<String>>` and filter it out of the render loop so the row/subtree disappears on click. Spawn the delete; on success the refetch confirms the absence (reconciliation is automatic since the server no longer returns it). On error, remove the id from `deleted_ids` to restore the row in place and `show_snackbar`. Only use this where the user stays on the page to see the removal; deletes that navigate away are out of scope.

## High-value opportunities

### Join speaker queue (SPEAK-1)

- Action: an authenticated user clicks Talk/Question/Clarify/Misunderstood/Procedure to add themselves to the queue.
- Today: `speak.rs:645-681` spawns `insert_node` and waits for `applied()` to bump `refresh` (`speak.rs:13-21`), which refetches the queue (`speak.rs:214-237`). No local change until the round-trip lands.
- Why optimistic helps: this is the most frequent and most latency-sensitive queue action, and the speaker sees silence ("did I join?") until the refetch returns.
- Implementation: recipe (a). A `Vec<PendingSpeaker> { key, name, type, index }` signal; the key is generated client-side at `speak.rs:651-655` (name-lowercase + `now_ms`), so it is collision-proof and reconciles directly. Render the pending row muted, priority-sorted, with a "sending" label. `reconcile_pending()` drops it when the real speaker with that key returns.
- Rollback/risk: low. Unique key, non-destructive; on error discard the pending row, restore focus to the join button, and snackbar. The user can rejoin immediately.

### Project agenda item to screen (ADMIN-1 / CONTENT-2)

- Action: the chair taps an agenda item (or the content-card Project button) to project a node to the room screen via `set_active_relation`.
- Today: subscription-driven. `admin.rs:186-202` calls `set_active_relation(ctx, Some(item_id))` and shows only a snackbar; the agenda row highlights only after the `relations(name: "active")` subscription (`admin.rs:45-50`) bumps and the active node refetches (`admin.rs:54-61`). The content-card path (`content.rs:189-209`) is the same, whereas the sibling `screen_comments` toggle (`content.rs:220-227`) already flips its signal locally.
- Why optimistic helps: chairs project 10 to 20+ items per meeting; the ~500ms subscription-plus-refetch delay before the row highlights is noticeable to fast clickers.
- Implementation: recipe (b). A `Signal<Option<String>>` optimistic "projected node id". On click, set it and highlight the row immediately (`is_active = active_id == item.id`, `admin.rs:163`). The existing subscription refetch reconciles it automatically. On error, clear the override and snackbar.
- Rollback/risk: low. The upsert is idempotent (`graphql.rs:741-762`); concurrent chairs resolve by server last-write-wins, and the next subscription push corrects any stale optimistic state.

### Cast vote (VOTE-1)

- Action: a member selects options and clicks Cast Vote.
- Today: `poll.rs:284-323` awaits `cast_vote`/`vote_cast_secret`, then does `refresh += 1`, which re-runs `already_voted` (`poll.rs:136-152`) and `tally` (`poll.rs:159-173`); a live subscription (`poll.rs:107-113`) also refetches. The ballot stays unchanged until the refetch lands.
- Why optimistic helps: voting is frequent and latency-sensitive; the member stares at their selected ballot for 1 to 3s on slow links before "you have voted" and the tally move.
- Implementation: recipe (a). Push a pending vote keyed by the voter to a pending `Vec`, render it in the tally so bars move at once (muted "sending" style), and `reconcile_pending()` drops it when the refetch returns a matching vote. One-vote-per-member is backend-enforced (UNIQUE on `parent_id+key`; secret ballots use a has-voted marker), so the cast is safe.
- Rollback/risk: medium. Precondition: the cast button has no `busy` guard (`poll.rs:468-473`), so rapid clicks can double-fire and the second fails with "hasVoted". Add a `busy` signal to disable the button during the cast before adding optimism. On rejection, retain the pending row, surface the error card, and leave the ballot intact so the user can retry.

### Invite member, single (MEMBER-1)

- Action: an owner/manager enters an email (or picks a user) and invites.
- Today: `member.rs:298-301` calls `invite_member()` then `bump_data_version()`, refetching the whole roster (`query_members_page`, `member.rs:58-68`). The new pending row appears only after the refetch.
- Why optimistic helps: single invites are frequent and latency-sensitive; the roster sits stale for the round-trip after a "sent" snackbar.
- Implementation: recipe (a). A `PendingMember { email, name, accepted: false, ... }` signal rendered as a muted `.member-row-pending` row. Reconcile by email (the schema has UNIQUE `members_parent_id_email_key`), dropping the pending row once the refetch returns that email. On paginated views, scope pending rows to the unfiltered "all" tab to avoid filter/pagination mismatch (see MEMBER-9 note).
- Rollback/risk: low. On failure remove the pending row and snackbar; navigating away before the refetch is acceptable (the invite was sent and shows on return). Backend enforces email uniqueness.

### Promote/demote owner (MEMBER-4)

- Action: after a confirm dialog, an owner toggles another member's owner flag.
- Today: `member.rs:456-461` builds `MembersSetInput { owner: Some(make_owner), ... }`; `apply_member_update` (`member.rs:692-698`) calls `update_member()` then `bump_data_version()`, refetching the roster before the star chip flips.
- Why optimistic helps: the user has already confirmed via dialog, so the star flip is the primary confirmation; waiting 1 to 2s after a confirmed action is friction.
- Implementation: recipe (b), reconciled by member id (simpler than by key). A `Signal<Option<(member_id, new_owner)>>` shadow that flips the chip immediately. On refetch, clear it if the fetched flag matches; on error, flip back and snackbar.
- Rollback/risk: low. Gated by context-owner permission, so rejection is rare; a two-manager race resolves last-write-wins on refetch.

### Accept invitation (HOME-ACCEPT-INVITE)

- Action: a user clicks Accept on an `InvitedContextItem` on the home page.
- Today: `home_list.rs:460-487` spawns `accept_invitation` (line 470) then `bump_data_version()` (line 484), refetching the whole home list; the invite stays visible for 1 to 2s.
- Why optimistic helps: accepting is a frequent onboarding action on a visible list, and the user stays on the home page (no navigation away), so the optimistic state is seen.
- Implementation: recipe (c). `invited_groups`/`invited_events` are derived at render time from fetched state (`home_list.rs:99-110`), so filtering is declarative. Add a `pending_accepted: Signal<Vec<String>>` of invite ids and skip matching invites. Reconciliation is automatic since the refetch no longer returns the invite.
- Rollback/risk: low. `bump_data_version()` is unconditional (line 484), so any server failure restores the invite via refetch; snackbar on `accept_invitation` returning false.

## Medium / Low

### Voting and polls (vote/poll.rs, vote/position.rs)

- Close poll (owner): `poll.rs:341-372`, recipe (b): shadow `open` with a local `closed` signal so the ballot hides and results show at once; revert on error. Medium.
- Add question: `position.rs:59-98`, recipe (a): stable key `format!("q{}", now)` at `position.rs:76`; render pending muted, restore text on error (mirror the error path at `position.rs:371-375`). Medium.
- Add candidate: `position.rs:333-379`, recipe (a): photo already uploaded before insert (`position.rs:309-330`), so the pending carousel card is complete; remove card and snackbar on error. Medium.
- Stop projecting (chair): `admin.rs:138-139`, recipe (b): clear `active_id` locally to hide the stop button and "on screen" chip; restore on error. Medium.
- Stop poll from admin console: `admin.rs:409-419`, recipe (b): a `pending_closed` set keyed by poll id flips the lock icon and hides the stop button; reconcile when the tally refetch (`admin.rs:328-342`) returns `mutable=false`. Medium.
- Focus section (chair): `admin.rs:224-267`, recipe (b): currently fire-and-forget with no feedback; a `Signal<Option<String>>` pending anchor highlights the chosen heading; the projector is subscription-driven so a wrong optimistic highlight never moves the room. Medium.

### Speaker queue (speak.rs)

- Move up/down (reorder): `speak.rs:390-429`, recipe (b): a `pending_reorder {speaker_id, new_index}` override applied inside `sorted_speakers()` (`speak.rs:754-763`); `busy` (`speak.rs:198-199`) prevents races; cleared on refetch/error. Medium.
- Remove speaker: `speak.rs:431-450`, recipe (c): filter the speaker from the local list; on error re-insert at its sorted position (index + type + createdAt) and snackbar. Medium (rollback restoring sort order is the main cost).
- Next speaker (advance): `speak.rs:470-497`, recipe (c): filter the first speaker and re-anchor the timer immediately; spawn `delete_node` + `move_timer` in one async block and clear pending only after both succeed to avoid divergence. Medium.
- Timer start/stop: `speak.rs:572-592`, recipe (b): optimistic `{time, updatedAt: now()}` drives `remaining_seconds` (`speak.rs:251-254`); revert on error. Medium.

### Members (member.rs)

- Invite by user node (autocomplete): `member.rs:262-270`, recipe (a): reconcile by node_id. Medium (guard the duplicate-invite case: composite unique `(parent_id, name, email, node_id)` means re-inviting silently fails; filter already-invited users from autocomplete or show "Already invited").
- Activate/deactivate: `member.rs:629-633`, recipe (b): flip the check icon on click (no confirm dialog); idempotent, revert on error. Medium.
- Hide/show: `member.rs:645-649`, recipe (b): flip the row's `.member-row-hidden` class/opacity (`member.rs:549`) immediately; track the toggle independently of page state on paginated rosters. Medium.
- Paginated-roster reconciliation: `member.rs:25-69`, supporting infra for MEMBER-1: scope optimistic pending rows to the "all" filter only; do not patch total/page counts on filtered tabs. Medium.
- Edit name/email: `member.rs:84-104`, recipe (b) by member id: store pre-edit values, apply new name/email locally, revert on server rejection (email uniqueness). Low (infrequent, full-page refetch latency).

### Folders and content (folder.rs)

- Lock/unlock folder: `folder.rs:290-322`, recipe (b): lift `attachable` to a local `Signal<bool>`, flip on click, revert on error. Medium (rare admin action).
- Paste / deep-copy: `folder.rs:323-360`, recipe (a): requires changing `deep_copy_node` to return the new node id (currently `let _ = deep_copy_node(...)` discards it) before pending rows can be keyed; reconcile by id. Low (signature refactor + mid-loop partial-failure handling).

### Home list (home_list.rs)

- Decline invitation: `home_list.rs:489-500`, recipe (c): reuse the accept pending-id signal to filter declined invites; restore on error. Medium (secondary to accept).

## Already optimistic / not worth it

Do not re-propose these:

- Post comment and post reply: already optimistic; this is the canonical reconcile-by-key pattern (`comments.rs:26-49`, `comments.rs:493-608`, reply signal at `comments.rs:287`). Reuse it, do not rebuild it.
- Toggle comments on screen: already optimistic (`content.rs:220-227` sets the signal on success). Minor nit: it does not roll back on error, but the snackbar covers it.
- All client-side preferences and view state: theme mode/seed/density (`theme.rs`, `density.rs`), folder grid/list (`folder.rs`), drawer tree expand, TOC popover, tools sheet, copy-selection (`SELECTED`). These are local-only signal writes, already instant.
- Editor formatting, links, and authors: local `richtext::exec` / signal edits until save (`editor.rs`); no per-action round-trip.
- Save document: skip: navigates away on success (`editor.rs:377-381`), so an optimistic state is never seen, and rollback would destroy contenteditable undo/caret state. Autosave is intentionally silent.
- Deletes that navigate away: delete document/file/folder/group/event (`content.rs:305-313`, `file.rs:159-218`, `folder.rs:386-416`): the page unmounts on success, so optimistic removal is invisible and rollback confusing.
- Create group/event and create context: skip: navigates into the new context immediately (`home_list.rs:342-350`), so the home-list refetch is never observed.
- Start poll / add amendment: skip: both navigate away to the new poll/editor (`poll.rs:692-745`, `position.rs:447-490`).
- Bulk invite import: skip: `insertMembers` returns only an affected-rows count, not ids (`graphql.rs:1064-1088`), so reconcile-by-key is infeasible; duplicate emails fail the whole transaction. The count toast already gives immediate feedback.
- Remove member: skip: rare, destructive, confirm-dialog-gated; a row that vanishes then reappears on rejection is more confusing than the current wait.
- Open/close speaker list: skip: the backend does not enforce `parent.mutable` on speak inserts, so an optimistic "closed" state could silently accept joins; needs backend enforcement or preemptive join-disabling first.
- Clear speaker queue: skip: destructive, confirm-gated, and the sequential delete loop breaks on first error, so partial-failure rollback is ambiguous. Bump-refetch is the correct fail-safe here.
- Projector view (ScreenApp/FollowApp): skip: passive, subscription-driven, on a different device; optimism is impossible and would break room trust.

## Suggested order

Build the shared infra first, then work biggest perceived-latency wins down. All of these reuse recipes (a)/(b)/(c), so a single generalized reconcile-by-key helper (lifted from `comments.rs:40-49` to accept any `{key}` row type) is the one piece to build once.

1. Shared reconcile-by-key helper (generalize `PendingComment`/`reconcile_pending` off comments) plus a "sending"/muted row style shared with `.comment-pending`.
2. Join speaker queue (SPEAK-1): most frequent, highest "did it work?" anxiety.
3. Cast vote (VOTE-1): add the missing `busy` guard on the cast button first, then optimism.
4. Project agenda item (ADMIN-1 / CONTENT-2): chairs project many items per meeting; apply the existing `screen_comments` local-signal shape.
5. Accept invitation (HOME-ACCEPT-INVITE) and decline (HOME-DECLINE-INVITE): one shared pending-id signal on the home list.
6. Member toggles: promote/demote (MEMBER-4), activate/deactivate (MEMBER-5), hide/show (MEMBER-6): one override-signal helper by member id covers all three; land single invite (MEMBER-1) alongside using the shared insert helper.
7. Folder-add / lock (FOLDER-1 add child, FOLDER-4 lock) and the remaining vote/position inserts (add question, add candidate): direct reuse of the insert and toggle recipes.
8. Remaining speaker and admin toggles (reorder, remove, next, timer, close poll, stop projecting, focus) as third-tier polish.

# wiki Dioxus — remaining work

`apps/wiki` is a Rust/Dioxus/WASM port of the former React app (which it has now
replaced), against the same NHost/Hasura backend (production
<https://radikal.wiki>; test with a real account — never commit credentials).

This plan lists **only what is still missing or partial** versus the React
original, plus features intentionally not ported (and why). It is the output of
two full component-by-component source audits (broad first pass + an exhaustive
second pass across GraphQL-operation / i18n-string / hooks / data-field /
dependency / route angles plus per-component deep-dives, each candidate
adversarially verified against the port). The port renders every screen, but
several **create / admin / permission / real-time / mobile** flows are absent —
the old "covers every flow end to end" claim was over-optimistic.

Already done (not repeated; see git history): the initial port of all screens,
GraphQL subscriptions/real-time, the rich editor + threaded comments, the extra
apps (graph/program/profile/social/redirect/parent/cow), pull-to-refresh, and
the Material Design 3 colour + theming system (replaceable M3 scheme from the
Radikale brand via `scripts/gen-theme.ts` → `assets/m3-theme.css`, plus
`assets/m3-tokens.css`).

## How to test

- **Unit** — `just test` (pure logic). Add one whenever a bug is wire-format or
  logic shaped.
- **Browser** — `just test-browser` (headless Firefox/WebDriver);
  `WIKI_EMAIL=… WIKI_PASSWORD=… just test-browser` adds authed checks against the
  live backend, `--shots` saves light/dark × desktop/mobile screenshots to
  `./screenshots` (read them — the contrast audit can't see layout/visual bugs).
- Per gap: reproduce the behaviour, build the fix here, diff the GraphQL, and
  lock it in with a unit test and/or a `test-browser.nu` assertion.

---

## 1. GraphQL surface completeness (foundational — unblocks much of §2–§6)

The cynic query fragments and `*_set_input` structs omit many fields the schema
has and React uses. Add them (grep `graphql/schema.graphql` for each):

- `[x]` **`isOwner` / `isContextOwner`** computed fields — done on the node
  fragments; drive the §2 owner-gating.
- `[x]` **`owner` relation object** (UserRef) — done on node fragments; used as a
  fallback author label on questions/candidates and in comments.
- `[x]` **`attachable`** on `NodeFields` (read) — done; gates the add-FAB. (Owner
  toggle of the lock, §6, is not yet wired.)
- `[x]` **`createdAt`** on `NodeWithChildren` (read) — done; drives the
  content/file "created N ago" subtitle. (Timestamp *editing* is not carried.)
- `[x]` **`contextId` / `ownerId` / `parentId`** on `NodesSetInput` — done.
- `[x]` **`MembersSetInput`**: `active`/`email`/`name`/`owner`/`parentId`;
  **`MemberFields`**: `active`/`email` — done; power the member admin (§5).
- `[x]` **Aggregate queries** — done: `count_nodes` via `nodesAggregate`, used for
  the poll-list vote-count badge. (Drawer expander + invite counts still don't use
  an aggregate.)
- `[x]` **Member ordering by display name** — done (client-side, case-insensitive
  in MemberApp; not the server `members_order_by.user`).

## 2. Permissions & owner-gating (frontend gates are missing) — DONE

The backend row permissions block unauthorized writes, but the UI showed owner-
only controls to everyone. Now gated behind `isContextOwner` / `is_owner` /
`mutable`, matching React.

- `[x]` **Folder admin controls** (sort, add-FAB) — done: sort gated on
  `is_context_owner`, the add-FAB on `is_auth && attachable`.
- `[x]` **Content edit / delete** — done: edit gated on owner + `mutable`, delete
  on owner (`is_owner || is_context_owner`).
- `[x]` **Add-content FAB** — done: gated on `is_auth && attachable`.

## 3. Voting & polls (can vote; cannot manage)

- `[x]` **Create / open a poll** (React `poll/PollDialog.tsx`) — done: owner
  StartPollButton (range, hide-result, For/Imod/Blank or candidates+Blank),
  closes the prior active poll, inserts `vote/poll`, sets the context `active`
  relation (`set_active_relation`), navigates to the ballot.
- `[x]` **Stop / close a poll** (React `poll/PollAdmin.tsx`) — done: owner "Stop"
  sets `mutable:false`. (Snapshotting eligible-voter count into `data.voters` is
  not carried; the tally reads live votes.)
- `[x]` **Position screen** (React `vote/PositionApp.tsx`) — done: PositionApp
  composes content + StartPollButton + candidate gallery + questions + polls.
- `[x]` **Candidate gallery + view** — done (display): a `vote/candidate` photo
  gallery (from `data.image`) linking to each candidate, which opens as content.
  (Creating a candidate with a photo upload is not yet offered.)
- `[x]` **Questions** (React `vote/QuestionList.tsx`, `AddQuestionButton.tsx`) —
  done: numbered `vote/question` list with add (`data.text`) and owner/author
  delete. (Question reorder/sort not wired.)
- `[x]` **Amendments** (React `vote/AddChangeButton.tsx`) — done: a "New amendment"
  button + name dialog on PolicyApp inserts a `vote/change` and redirects to its
  editor.
- `[x]` **Live-update on poll open** — done: VoteApp subscribes to the context
  `active` relation so a newly-opened poll appears without reload.
- `[x]` **Poll-list affordances** — done: a vote-count badge on poll rows
  (PollVoteBadge via the aggregate). (Created-at + owner delete on poll rows are
  not carried.)
- `[x]` **`hideResult`** (`data.hidden`) — done: a hide-result poll shows tallies
  only to the context owner; others see options + a "results hidden" note.
- `[x]` **Voting-rights status** — done: a card on VoteApp reads "you have / do not
  have voting rights" from `is_active_member` (the port's canVote).
- `[~]` Owner **Sort** entry buttons in vote/comment/question lists; block
  over-selection live on checkbox (not only at submit).

## 4. Content, editor & files

- `[x]` **File upload** (React `util/FileUploader.tsx`) — done: `nhost::upload_file`
  (multipart POST to storage) + a File option in the add-content modal that
  uploads and inserts a `wiki/file` node with `{ fileId, type }`.
- `[x]` **Add-content dialog** — done (file option): `FolderAdd` now offers
  document/folder/**file** (with upload). (Free-text body for question/comment is
  handled in PositionApp/comments; a duplicate-name check is still absent, and it
  still does not read the node's `inserts` list.)
- `[blocked]` **Context creation + permission seeding** (`contextPerm`) —
  investigated end-to-end: the `nodes` insert `_check` on the **user** role denies
  creating a `wiki/group`/`wiki/event` (verified live: user is denied, the admin
  role succeeds), and the `contextPerm` template grants no group/event insert.
  So context creation is an **admin-role backend operation**, not a regular-user
  feature (which is why React also omits group/event from its add dialog). The
  seeding mechanism (`create_context_node` + `seed_context_permissions`, with
  `insertPermissions` validated live) is understood but can't be surfaced to
  users without a backend permission change. Not shippable as a user feature.
- `[x]` **File-node toolbar + sub-content** — done: a file page now has an owner
  delete (confirm, members-first) and a comment section under the viewer. (Edit /
  members-chips / publish on a file are not carried; the viewer already had
  download + created-at.)
- `[x]` **Raw-file download** (React `content/DownloadButton.tsx`) — done: a header
  Download button on FileApp for every previewable type.
- `[x]` **Cover image lost on save** — done: the editor preserves the node's other
  `data` keys (e.g. `image`) and overwrites only `content`. (A cover-image
  uploader/preview in the editor is still not offered.)
- `[x]` **Group/event double-card** — done: a CommentSection now renders below the
  folder listing for `wiki/group`/`wiki/event` (the description already shows in
  the folder card). The metadata/members card is not separately duplicated.
- `[x]` **Delete removes member rows first** (React `DeleteButton.tsx:16-19`) —
  done: content delete now removes the node's member rows first
  (`delete_node_members`) so no orphans are left.
- `[x]` **Code mark is broken** — done: the code button now toggles — when the
  caret is inside a `<code>` span it unwraps to plain text instead of nesting.
- `[x]` **Submit/publish confirmation** — done: the editor's Submit now routes
  through a confirm dialog carrying the submitWarning before making the node
  immutable. (A publish button on the read view is still not added.)
- `[x]` **Editor paste fidelity** — done: a paste handler sanitizes clipboard HTML
  to the editor's semantic subset (keep b/i/u/a/code/headings/lists, unwrap the
  rest, drop script/style), with a plain-text fallback.
- `[x]` **PDF viewer** — done: the iframe fills the viewport height
  (calc(100vh - 160px), min 480px) instead of a fixed 80vh. (Still the native
  iframe, not the Google-Docs viewer.)
- `[x]` **`createdAt` subtitle** — done: a compact relative time (full date in the
  tooltip) on content/file headers. Editor `DatePicker` for `createdAt` is niche
  and not carried. **minor**
- `[x]` Editor / media polish — done: strip a leading empty paragraph on save
  (unit-tested), and a responsive width for video/audio (video max-height 70vh).
  (An editor error fallback and media `autoPlay` are not carried; autoplay is
  browser-gated and intrusive.)
- `[x]` **Bulk member import** / **xlsx SheetReader** — done: an owner .xlsx file
  input imports a Fornavn/Efternavn/Email roster via the pure-Rust `calamine`
  parser (wasm32) + a bulk `insertMembers`. End-to-end validated.

## 5. Members & invites

- `[x]` **Member administration** (React `member/MembersDataGrid.tsx`) — done:
  per-row promote/demote owner, mark active/inactive, hide/show, edit name+email,
  and remove (confirm), via `update_member`/`remove_member`.
- `[x]` **Invite existing users by name** — done: a users `displayName _ilike`
  autocomplete (race-guarded) in MemberApp; clicking a match invites by `nodeId`
  (binds nodeId + name) via `invite_member_by_node`. (Single-select, not multi.)
- `[x]` **Members entry point** — done: a Members entry in the app rail + mobile
  app bar (the component gates admin actions itself).
- `[x]` **Accept-invite unique-constraint fallback** — done: on accept failure,
  accept the existing `(parent, node)` membership (`accept_existing_member`) then
  drop the duplicate invite — ordered so a transient failure never loses the
  invite (safer than React).

## 6. Folder management

- `[x]` **Copy / paste** — done: a per-item copy toggle feeds a clipboard
  (GlobalSignal); a Paste button on any folder recursively deep-duplicates each
  selected node + its members + subtree (`deep_copy_node`), guarded against
  pasting into itself/a descendant (`is_descendant_of`). (Single-list select, not
  a full multi-folder session model.)
- `[x]` **Folder lock** (`attachable`) — done: a context owner toggles whether
  children can be added via a header lock button (update_node on `attachable`).

## 7. Real-time

- `[x]` **Projector live-update** — done: `ScreenApp` subscribes to the context
  `active` relation, so the projected pane changes when the active node is
  switched remotely.
- (Voter poll-open subscription is in §3.)

## 8. Mobile & responsive

- `[x]` **Mobile app navigation** (React `layout/MobileMenu.tsx`) — done: a
  floating mobile app bar (bottom-left) exposes the same context apps as the
  desktop rail, so Speak/Vote/Members are reachable on a phone.
- `[x]` **Home list on mobile** — done: HomeList (groups/events) renders in the
  Home main pane on mobile (`.home-mobile-list`), where the drawer is hidden.
- `[x]` **Pending-invite badge** — done: a count badge on the Home nav item (rail
  + mobile bar), fed by a `PENDING_INVITES` global resolved in Layout.

## 9. Search, auth & routing

- `[x]` **`?type=passwordReset` deep-link** — done: the token is captured in
  `main()` before the router drops the query, exchanged for a session, and the
  set-password form is shown.
- `[x]` **`?app=screen` chrome** — done: Layout renders the projector view
  full-screen (`.screen-full`), with no drawer/rail/bar.
- `[x]` **Clear GraphQL cache on login/logout** — done: bump_data_version on both
  so all resources refetch and no previous-session data lingers.
- `[x]` **Resend verification email** — done: an unverified sign-in offers a
  resend button on the login screen (`nhost::send_verification_email`).
- `[x]` **Public user profile** — done: `wiki/user` routes to a UserApp showing the
  person's groups + events. (Authored-content deep links not carried.)
- `[x]` **Search** — mostly done: Ctrl-K, arrow-key/Enter nav, the hidden/orphan
  filters (`parent not null`, `mime.hidden=false OR mime.context=true`), and the
  parent-name secondary line. (Context-scoping to the current context is not
  carried; search stays global.)
- `[~]` Auth minor: intentionally left. Unverified auto-redirect-once-verified
  needs polling; post-login return-to-origin (`navigate(-1)`) risks leaving the
  SPA on a deep-link login; the status-100 already-logged-in case is marginal.
  Low value / high risk relative to the churn. **minor**

## 10. Speaker list & smaller UX / a11y / polish

- `[x]` **Speaker join types** — done: restored the 5th ("misunderstood") type and
  renumbered procedure back to `4`, matching React + the existing `data:"4"` rows;
  the queue order (data desc) is unchanged.
- `[x]` **Icon-button a11y** — done: aria-label added to the icon-only buttons that
  had no accessible name (menu/search/close/expand, comment reply/send, chip
  remove).
- `[x]` **Snackbar** — done: a stack of up to 3 (maxSnack) with preventDuplicate,
  bottom-centre clear of the drawer/rail.
- `[x]` **ZoomableImage** — done: fade-in on mount + a broken-image error state.
- `[x]` **SortApp** — done: seeded from `visible_sorted`, so hidden mimes no
  longer appear in the sort list. (A forced fresh fetch is not added; it reads the
  passed-in node.)
- `[~]` Polish: done — theme-color synced to the active scheme, `document.title` set
  to the node name, the folder rail stays active during `?app=editor`/`?app=sort`,
  the ballot blocks over-selection live, breadcrumb click scrolls to top, and the
  desktop drawer has rounded right corners. Intentionally left: drawer
  close-on-mouseleave (bad UX for an always-visible drawer), Danish relative-time
  (already `m/h/d`, language-neutral), per-app scroll restore + batching the
  sort-save mutations (marginal, both already work). **minor**

---

## Open backlog (wiki GitHub issues still relevant)

- `[ ]` `#138` Replace "questions" with a comment model (see §3 Questions).
- `[ ]` `#25` Content metadata attributes (e.g. a "keep longer" flag).
- `[ ]` `#69` Node revision / history table.
- `[ ]` `#115` Live collaborative editing.
- `[~]` `#128` Audio/MIDI — audio/video preview natively; MIDI needs an external
  JS synth + soundfont (CDN dependency).
- `[ ]` `#41` Export event participants.
- `[ ]` `#134` "Open" contexts anyone can join.
- `[ ]` `#147` New permission system + editing UI (informs the perm app; see §2).
- `[ ]` `#145` Error / stacktrace reporting API (relates to §9 error boundary).
- `[~]` `#33` PWA offline — installable; full offline needs the service worker at
  the site **root** (`/sw.js` or `Service-Worker-Allowed: /`), a deploy config.
- `[~]` `#139` Native notifications — "your turn to speak" fires; poll-open still
  to do.
- `[blocked]` `#108` Drive icons/apps from mime data — the `mimes.icon` is only a
  letter/number/questionmark avatar-mode hint; needs backend data first.
- **Need your input:** `#123` MimeAvatar-path-on-screen, `#155` nhost alternative,
  `#135`/`#136` native DB primitives, `#153` register campaign activity.

## Known issues

- **Release build won't load in Servo** (`Module fetching failed`); debug build
  runs there. Loads in Chrome/Firefox. Uninvestigated, low priority.
- ~~Folder-letter avatar contrast~~ — FIXED: the overlaid letter now uses
  `--md-on-primary` (the avatar's compliant white) with a shadow, so it no longer
  matches the green avatar background (audit: 0 violations).

## Intentionally excluded (React features that don't make sense to port)

Verified by the audit as dead code, intentional divergences, or already covered:

- **ResultDataGrid, PollChart, PollChartSub** — commented-out DevExpress dead code
  in React; the port renders a live results table (`admin.rs`) + per-option bars.
- **Permission-editing UI** (`PermApp`/`PermList`) — `null`/commented in React; the
  port's read-only `perm.rs` is ahead. (Editing is issue #147, §2.)
- **`NodeApp` force-directed graph** (`?app=node`) — commented-out React dead code.
- **`checkUnique` / `node.data()` computed fields** — the port achieves the same
  via `count_user_votes` and manual JSONB deserialization.
- **`HTMLtoDOCX` export** — the port exports `.odt` (intentional, no WASM DOCX lib).
- **OldBrowser version gate** — a WASM build can't start on the browsers it warned
  about, so the gate is moot.
- **HideOnScroll** auto-hide bar, **RightDrawer** chrome, **SpeedDial** join FAB,
  **MUI drawer slide animation**, **"current item" drawer jump** (`&& false`) —
  heavy/decorative MUI patterns; actions relocated, no functionality lost.
- **Slate.js engine** — reimplemented over `contenteditable`/`execCommand` with
  Slate↔HTML round-tripping (functional parity; only caveats: rich-paste fidelity
  §4 and the code-mark toggle bug §4).
- The port is **ahead** of React in: speaker admin (reorder + your-turn
  notifications), token refresh (single-flight, cross-tab, visibility nudge),
  the cross-context "newest content" feed, and the graph/program/profile/social/
  redirect/parent apps.

## Cross-cutting checks

- **GraphQL correctness:** filtered queries must omit unset fields (Hasura rejects
  `null` comparison expressions). Prefer `..Default::default()` +
  `skip_serializing_if`. Several §1–§6 gaps are just missing struct fields.
- **Permissions:** queries run with the user token; compare row visibility with
  `apps/wiki` for the same account. Frontend gating (§2) should mirror the backend.
- **Field naming:** cynic maps snake_case Rust → camelCase GraphQL
  (`mime_id` → `mimeId`); the schema is camelCase.

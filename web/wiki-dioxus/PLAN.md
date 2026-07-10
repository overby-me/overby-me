# RadikalWiki Dioxus — remaining work

`web/wiki-dioxus` is a Rust/Dioxus/WASM port of the React app in
[`web/wiki`](../wiki), against the same NHost/Hasura backend (production
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
- Per gap: reproduce in `web/wiki`, build it in `wiki-dioxus`, diff the GraphQL,
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
- `[ ]` **Aggregate queries** (`children_aggregate`, `membersAggregate`) — React
  uses counts for the drawer expander/skeleton, the poll-list vote-count badge,
  and invite counts. **medium**
- `[ ]` **Member ordering by `user.displayName`** (`members_order_by.user`).
  **minor**

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
- `[ ]` **Amendments** (React `vote/AddChangeButton.tsx`) — "new amendment" button
  (gated on `inserts` ∋ `vote/change`) → editor redirect, name-prefill under a
  position. **major**
- `[x]` **Live-update on poll open** — done: VoteApp subscribes to the context
  `active` relation so a newly-opened poll appears without reload.
- `[~]` **Poll-list affordances** (React `poll/PollList.tsx`) — poll rows lack the
  vote-count badge, created-at, and owner delete.
- `[x]` **`hideResult`** (`data.hidden`) — done: a hide-result poll shows tallies
  only to the context owner; others see options + a "results hidden" note.
- `[~]` **Voting-rights status** (React `vote/VoteApp.tsx`) — no `canVote` check
  or "you (do not) have voting rights / have (not) voted" card.
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
- `[ ]` **Context creation + permission seeding** (`contextPerm`) — a new group/
  event/context is created with a plain `insert_node` and gets no permission rows.
  (Note: the port has no context-creation flow yet, so this is latent.) **major**
- `[ ]` **File-node toolbar + sub-content** (React `file/FileApp.tsx` +
  `ContentToolbar`) — a file page shows the viewer + download + created-at, but
  still no delete / edit / members / publish, no member chips, no comments/
  changes/questions. **major**
- `[x]` **Raw-file download** (React `content/DownloadButton.tsx`) — done: a header
  Download button on FileApp for every previewable type.
- `[x]` **Cover image lost on save** — done: the editor preserves the node's other
  `data` keys (e.g. `image`) and overwrites only `content`. (A cover-image
  uploader/preview in the editor is still not offered.)
- `[ ]` **Group/event double-card** — React stacks `ContentApp(hideMembers)` above
  `FolderApp` for `wiki/group`/`wiki/event` (metadata + comments card, then the
  folder listing); the port renders only the folder. **medium**
- `[x]` **Delete removes member rows first** (React `DeleteButton.tsx:16-19`) —
  done: content delete now removes the node's member rows first
  (`delete_node_members`) so no orphans are left.
- `[x]` **Code mark is broken** — done: the code button now toggles — when the
  caret is inside a `<code>` span it unwraps to plain text instead of nesting.
- `[~]` **Submit/publish confirmation** (React `PublishButton.tsx`) — submit fires
  with no confirm dialog / `submitWarning` for an irreversible action (i18n keys
  exist, unused); also no publish button on the read view.
- `[~]` **Editor paste fidelity** — no custom HTML-paste deserializer (React
  `Slate.tsx` `withHtml`/`deserialize`); paste is left to the browser. **medium**
- `[~]` **PDF viewer** — direct-URL iframe at a fixed `80vh`; React wraps in the
  Google-Docs viewer with dynamic height. **medium**
- `[x]` **`createdAt` subtitle** — done: a compact relative time (full date in the
  tooltip) on content/file headers. Editor `DatePicker` for `createdAt` is niche
  and not carried. **minor**
- `[ ]` Editor: strip the empty leading paragraph on save; add an editor error
  fallback; audio/video `autoPlay` + video width. **minor**
- `[~]` **Bulk member import** / **xlsx SheetReader** (React `invite/InvitesFab.tsx`)
  — import a Fornavn/Efternavn/Email roster from `.xlsx`. Heaviest lift (needs a
  WASM xlsx parser); reasonable to defer.

## 5. Members & invites

- `[x]` **Member administration** (React `member/MembersDataGrid.tsx`) — done:
  per-row promote/demote owner, mark active/inactive, hide/show, edit name+email,
  and remove (confirm), via `update_member`/`remove_member`.
- `[ ]` **Invite existing users by name** (React `invite/InvitesTextField.tsx`) —
  only single-email invites; add a `users` `displayName _ilike` autocomplete with
  multi-select, binding `nodeId` for known users (`invite_member` currently sends
  no `nodeId`/`name`). **major**
- `[x]` **Members entry point** — done: a Members entry in the app rail + mobile
  app bar (the component gates admin actions itself).
- `[ ]` **Accept-invite unique-constraint fallback** — if a member row already
  exists, React deletes the placeholder email-invite row and updates the existing
  membership; the port has no fallback and can hit the unique constraint. **major**

## 6. Folder management

- `[ ]` **Copy / paste** (React `folder/FolderDial.tsx` + `FolderList` checkboxes +
  `session.selected`) — select items, recursively deep-duplicate node + children +
  members, with a `checkIfSuperParent` circular-parent guard. Entirely absent.
  **major**
- `[ ]` **Folder lock** (`attachable`) — read the lock state and let a context
  owner toggle whether children can be added (needs the `attachable` field, §1).
  **major**

## 7. Real-time

- `[ ]` **Projector live-update** — `ScreenApp` doesn't subscribe to the `active`
  relation, so the projected content pane doesn't change when the active node is
  switched remotely (React `ScreenApp` uses `useSubsGet`). **major**
- (Voter poll-open subscription is in §3.)

## 8. Mobile & responsive

- `[x]` **Mobile app navigation** (React `layout/MobileMenu.tsx`) — done: a
  floating mobile app bar (bottom-left) exposes the same context apps as the
  desktop rail, so Speak/Vote/Members are reachable on a phone.
- `[ ]` **Home list on mobile** — authed mobile users see only the welcome card in
  the main pane; React also renders `HomeList` (groups/events) there. **medium**
- `[ ]` **Pending-invite badge** on the Home rail/nav item (data already exists via
  `query_invitations`). **medium**

## 9. Search, auth & routing

- `[x]` **`?type=passwordReset` deep-link** — done: the token is captured in
  `main()` before the router drops the query, exchanged for a session, and the
  set-password form is shown.
- `[~]` **`?app=screen` chrome** — the projector view still shows the drawer/rail/
  bar; it should render full-screen. **major**
- `[~]` **Clear GraphQL cache on login/logout** — the port relies on token-change
  refetch; React explicitly clears the cache to avoid cross-session stale data.
- `[ ]` **Resend verification email** on an unverified sign-in (React
  `auth/AuthForm.tsx`; `nhost.rs` has no `send_verification_email`). **medium**
- `[~]` **Public user profile** (React `layout/UserApp.tsx`) — `wiki/user` isn't
  routed (falls to `NodeApp`); `ProfileApp` is self-only and omits the authored-
  content list. Route `wiki/user` → profile; show any user's memberships/events/
  authored content.
- `[~]` **Search** (React `layout/SearchField.tsx`) — add context-scoping (filter
  by `contextId`), the `mime.hidden=false OR mime.context=true` + `parent not
  null` filters (currently shows hidden/orphan nodes), a **Ctrl-K** shortcut,
  arrow-key + Enter result nav, and the parent-name secondary line.
- `[ ]` Auth minor: unverified page should auto-redirect once verified; handle the
  already-logged-in sign-in error (status 100); post-login return-to-origin
  (`navigate(-1)` vs always Home). **minor**

## 10. Speaker list & smaller UX / a11y / polish

- `[~]` **Speaker join types** (React `speak/avatars.tsx`) — the port has 4 of 5
  (drops "misunderstood/announcement" and renumbers procedure `4`→`3`, diverging
  from `order_by data desc` and existing `data:"4"` rows). Restore or migrate.
- `[ ]` **Icon-button a11y** — icon-only buttons use `title` not `aria-label`;
  6 have no accessible label. **medium**
- `[ ]` **Snackbar** — no `maxSnack` stacking, `preventDuplicate`, or drawer-aware
  positioning (single centered message only). **medium**
- `[ ]` **ZoomableImage** — no loading/error state or fade-in (only click-to-zoom).
  **medium**
- `[~]` **SortApp** — doesn't filter hidden mimes (`visible_sorted`) or force a
  fresh fetch.
- `[ ]` Polish: sync the `meta[name=theme-color]` to the active light/dark scheme
  (currently a static value); set `document.title` to the active node name;
  breadcrumb re-click should scroll content to top; drawer rounded right corners +
  close-on-mouseleave; Danish relative-time localization; folder rail stays active
  during `?app=editor` and vote during `?app=poll`; per-app scroll-position
  restore; batch the sort-save mutations. **minor**

---

## Open backlog (RadikalWiki GitHub issues still relevant)

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
  `web/wiki` for the same account. Frontend gating (§2) should mirror the backend.
- **Field naming:** cynic maps snake_case Rust → camelCase GraphQL
  (`mime_id` → `mimeId`); the schema is camelCase.

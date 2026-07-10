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
- `[x]` **Amendments** (React `vote/AddChangeButton.tsx`) — done: a "New amendment"
  button + name dialog on PolicyApp inserts a `vote/change` and redirects to its
  editor.
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
- `[~]` **Clear GraphQL cache on login/logout** — the port relies on token-change
  refetch; React explicitly clears the cache to avoid cross-session stale data.
- `[x]` **Resend verification email** — done: an unverified sign-in offers a
  resend button on the login screen (`nhost::send_verification_email`).
- `[~]` **Public user profile** (React `layout/UserApp.tsx`) — `wiki/user` isn't
  routed (falls to `NodeApp`); `ProfileApp` is self-only and omits the authored-
  content list. Route `wiki/user` → profile; show any user's memberships/events/
  authored content.
- `[~]` **Search** — Ctrl-K shortcut + arrow-key/Enter result nav are done.
  Remaining: context-scoping (filter by `contextId`), the `mime.hidden=false OR
  mime.context=true` + `parent not null` filters (still shows hidden/orphan
  nodes), and the parent-name secondary line.
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
- `[x]` **SortApp** — done: seeded from `visible_sorted`, so hidden mimes no
  longer appear in the sort list. (A forced fresh fetch is not added; it reads the
  passed-in node.)
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
- **Folder-letter avatar contrast** — a folder whose name hashes to an avatar
  colour close to its text can fail the 1:1 contrast audit (seen on a scratch
  folder). The letter colour derivation should guarantee a contrast floor.

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

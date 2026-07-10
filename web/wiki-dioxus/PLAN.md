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

- `[ ]` **`isOwner` / `isContextOwner`** computed fields on node fragments
  (`schema.graphql:6620,6625`). The port fakes `isOwner` with `owner_id ==
  user.id` and can't express `isContextOwner` at all — the root cause of the
  missing owner-gating in §2. **major**
- `[ ]` **`owner` relation object** (UserRef) on node fragments — needed to show
  a creator's display name as a fallback label on questions/amendments/candidates
  and in threaded comments. **medium**
- `[ ]` **`attachable`** on `NodeFields`/`NodesSetInput` — the folder "lock"
  flag (read + toggle). See §6. **major**
- `[ ]` **`createdAt` / `updatedAt`** on `NodesSetInput`/`NodesInsertInput`
  (write) and `createdAt` on `NodeWithChildren` (read) — blocks timestamp editing
  and the content-header "created N ago" subtitle. **major**
- `[ ]` **`contextId` / `ownerId` / `parentId`** on `NodesSetInput` — the
  insert-then-self-reference update used when creating context nodes. **major**
- `[ ]` **`MembersSetInput`**: add `active`, `email`, `name`, `owner`,
  `parentId`; **`MemberFields`**: add `active`, `email`; **`UserRef`**: add
  `email` — enables member admin (§4). **major**
- `[ ]` **Aggregate queries** (`children_aggregate`, `membersAggregate`) — React
  uses counts for the drawer expander/skeleton, the poll-list vote-count badge,
  and invite counts. **medium**
- `[ ]` **Member ordering by `user.displayName`** (`members_order_by.user`).
  **minor**

## 2. Permissions & owner-gating (frontend gates are missing)

The backend row permissions block unauthorized writes, but the UI shows owner-
only controls to everyone (confusing, and buttons that then fail). React hides
whole panels behind `isContextOwner` / `owner` / `mutable`.

- `[ ]` **Folder admin controls** (export, sort, add-FAB) are shown to any authed
  user; React `FolderDial.tsx:48` returns `null` unless `isContextOwner`. **major**
- `[ ]` **Content edit / delete** buttons show without owner/`mutable` checks
  (React `ContentToolbar`/`DeleteButton` gate on owner + mutable). **major**
- `[ ]` **Add-content FAB** is gated only on `is_auth`, not `isContextOwner` /
  `attachable`. **major**
- (Depends on the `isOwner`/`isContextOwner`/`attachable` fields in §1.)

## 3. Voting & polls (can vote; cannot manage)

- `[ ]` **Create / open a poll** (React `poll/PollDialog.tsx`) — no way to start
  one (`FolderAdd` offers only document/folder). Needs min/max range, hide-result
  toggle, options (For/Imod/Blank or candidates+Blank), close prior active poll,
  insert `vote/poll`, **set the context `active` relation** (no such mutation
  yet). **major**
- `[ ]` **Stop / close a poll** (React `poll/PollAdmin.tsx`) — owner "Stop" sets
  `mutable:false` and snapshots eligible-voter count into `data.voters`. Also fix
  `vote.rs` misusing `poll.managePoll` as the voter subtitle. **major**
- `[ ]` **Position screen** (React `vote/PositionApp.tsx`) — `vote/position` falls
  to the generic node view; should compose content + candidates + questions +
  polls. **major**
- `[ ]` **Candidate gallery + view** (React `vote/CandidateList.tsx`,
  `CandidateApp.tsx`) — image gallery of `vote/candidate` (photo from `data.image`,
  per-user visibility); candidate opens as content with members hidden. **major**
- `[ ]` **Questions** (React `vote/QuestionList.tsx`, `AddQuestionButton.tsx`) —
  numbered `vote/question` list, add (gated on `inserts`), owner delete/sort.
  (Overlaps issue #138.) **major**
- `[ ]` **Amendments** (React `vote/AddChangeButton.tsx`) — "new amendment" button
  (gated on `inserts` ∋ `vote/change`) → editor redirect, name-prefill under a
  position. **major**
- `[ ]` **Live-update on poll open** — the voter's `?app=vote` view doesn't
  subscribe to the context `active` relation, so a newly-opened poll doesn't
  appear without reload (the `use_live` pattern exists elsewhere). **major**
- `[~]` **Poll-list affordances** (React `poll/PollList.tsx`) — poll rows lack the
  vote-count badge, created-at, and owner delete.
- `[~]` **`hideResult`** (`data.hidden`) not read: a closed hide-result poll
  should hide options/tallies from non-owners.
- `[~]` **Voting-rights status** (React `vote/VoteApp.tsx`) — no `canVote` check
  or "you (do not) have voting rights / have (not) voted" card.
- `[~]` Owner **Sort** entry buttons in vote/comment/question lists; block
  over-selection live on checkbox (not only at submit).

## 4. Content, editor & files

- `[ ]` **File upload** (React `util/FileUploader.tsx` via `nhost.storage.upload`)
  — `nhost.rs` has no upload path, so `wiki/file` nodes and document cover images
  can't be created. Needs multipart upload + presigned-URL helper. **major**
- `[~]` **Add-content dialog** (React `content/AddContentDialog.tsx`) — `FolderAdd`
  hardcodes document/folder and ignores the node's `inserts` field; add the
  file-upload option, free-text body for `vote/question`/`vote/comment`, and a
  duplicate-name check.
- `[ ]` **Context creation + permission seeding** (`contextPerm`) — a new group/
  event/context is created with a plain `insert_node` and gets no permission rows.
  **major**
- `[ ]` **File-node toolbar + sub-content** (React `file/FileApp.tsx` +
  `ContentToolbar`) — a file page shows only the viewer: no delete / download-raw /
  edit / members / publish, no member chips, no comments/changes/questions. **major**
- `[ ]` **Raw-file download** (React `content/DownloadButton.tsx`) via a presigned
  URL, for every previewable type (`.odt` export already exists for documents).
- `[ ]` **Cover image lost on save** — the editor never reads/writes `data.image`,
  so editing a document with a cover image **drops it** (ContentApp still renders
  one if present). Add the image field to save + an uploader/preview. **medium**
- `[ ]` **Group/event double-card** — React stacks `ContentApp(hideMembers)` above
  `FolderApp` for `wiki/group`/`wiki/event` (metadata + comments card, then the
  folder listing); the port renders only the folder. **medium**
- `[ ]` **Delete removes member rows first** (React `DeleteButton.tsx:16-19`) — the
  port deletes the node but not its `members`, leaving orphan rows. **medium**
- `[ ]` **Code mark is broken** — the port wraps the selection in `<code>` via
  `insertHTML` on every click (nests `<code>`), instead of toggling the mark off.
  **medium**
- `[~]` **Submit/publish confirmation** (React `PublishButton.tsx`) — submit fires
  with no confirm dialog / `submitWarning` for an irreversible action (i18n keys
  exist, unused); also no publish button on the read view.
- `[~]` **Editor paste fidelity** — no custom HTML-paste deserializer (React
  `Slate.tsx` `withHtml`/`deserialize`); paste is left to the browser. **medium**
- `[~]` **PDF viewer** — direct-URL iframe at a fixed `80vh`; React wraps in the
  Google-Docs viewer with dynamic height. **medium**
- `[ ]` **`createdAt` subtitle** ("created N ago" + tooltip) on content/file
  headers. Editor `DatePicker` for `createdAt` (context-owner) is niche. **minor**
- `[ ]` Editor: strip the empty leading paragraph on save; add an editor error
  fallback; audio/video `autoPlay` + video width. **minor**
- `[~]` **Bulk member import** / **xlsx SheetReader** (React `invite/InvitesFab.tsx`)
  — import a Fornavn/Efternavn/Email roster from `.xlsx`. Heaviest lift (needs a
  WASM xlsx parser); reasonable to defer.

## 5. Members & invites

- `[ ]` **Member administration** (React `member/MembersDataGrid.tsx`) — owners can
  only hide/unhide; add promote/demote **owner**, toggle **active**, edit name/
  email, and **remove a member** (`delete_member` exists, no button). Enabled by
  the `MembersSetInput` fields in §1. **major**
- `[ ]` **Invite existing users by name** (React `invite/InvitesTextField.tsx`) —
  only single-email invites; add a `users` `displayName _ilike` autocomplete with
  multi-select, binding `nodeId` for known users (`invite_member` currently sends
  no `nodeId`/`name`). **major**
- `[ ]` **Members entry point** — no owner "Members" button on a group/event view;
  `?app=member` is reachable only by typing the URL. **major**
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

- `[ ]` **Mobile app navigation** (React `layout/MobileMenu.tsx`) — the app rail is
  desktop-only, so on a phone **Speak/Vote/other apps are unreachable** (drawer
  only offers context + Home). Events are attended on phones. (Distinct from the
  skipped *styling* redesign #158 — this is reachability.) **major**
- `[ ]` **Home list on mobile** — authed mobile users see only the welcome card in
  the main pane; React also renders `HomeList` (groups/events) there. **medium**
- `[ ]` **Pending-invite badge** on the Home rail/nav item (data already exists via
  `query_invitations`). **medium**

## 9. Search, auth & routing

- `[ ]` **`?type=passwordReset` deep-link** — password-reset emails link to
  `/?type=passwordReset`; the port renders home instead of the set-password form.
  **major**
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

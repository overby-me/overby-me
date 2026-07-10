# RadikalWiki Dioxus — remaining work

`web/wiki-dioxus` is a Rust/Dioxus/WASM port of the React app in
[`web/wiki`](../wiki), against the same NHost/Hasura backend (production
<https://radikal.wiki>; test with a real account — never commit credentials).

This plan lists **only what is still missing or partial** versus the React
original, plus features intentionally not ported (and why). It was reconciled
against the React source by a full component-by-component audit; the previous
"everything is done" parity list was over-optimistic — the port renders every
screen, but several **create / admin / mobile** flows are absent.

Already done (not repeated here; see git history): the initial port of all
screens, GraphQL subscriptions / real-time, the rich editor + threaded comments,
the extra apps (graph/program/profile/social/redirect/parent/cow), pull-to-refresh,
and the Material Design 3 colour + theming system (a replaceable M3 scheme
generated from the Radikale brand by `scripts/gen-theme.ts` → `assets/m3-theme.css`,
plus `assets/m3-tokens.css` for shape/elevation/type/state/motion).

## How to test

- **Unit** — `just test` (pure logic: GraphQL filter serialization, path/icon/
  ordering helpers). Add one whenever a bug is wire-format or logic shaped.
- **Browser** — `just test-browser` drives the real app in headless Firefox over
  WebDriver; `WIKI_EMAIL=… WIKI_PASSWORD=… just test-browser` adds authenticated
  checks against the live backend, and `--shots` saves light/dark × desktop/mobile
  screenshots of home/context/vote/editor/speak to `./screenshots` (read them —
  the contrast audit can't see layout/visual bugs).
- Workflow per gap: reproduce in `web/wiki`, build it in `wiki-dioxus`, diff the
  GraphQL, then lock it in with a unit test and/or a `test-browser.nu` assertion.

## Parity gaps (found by the React↔Dioxus source audit)

`[ ]` open · `[~]` partial. Ordered by impact within each area.

### Voting & polls — the port can *vote* but not *manage* polls

- `[ ]` **Create / open a poll** (React `poll/PollDialog.tsx`). No way to start a
  poll in the port: `FolderAdd` only offers document/folder. Needs the dialog
  (min/max vote-range, hide-result toggle, options built from For/Imod/Blank for
  policy/change or from `vote/candidate` children + Blank for positions), closing
  any prior active poll (`update mutable:false`), inserting `vote/poll`, and
  **setting the context `active` relation** (no set-active-relation mutation
  exists yet). Without this, a poll can only be opened from the React app. **Biggest gap.**
- `[ ]` **Stop / close a poll** (React `poll/PollAdmin.tsx`). Owner-only "Stop"
  that sets `mutable:false` and snapshots the eligible-voter count into
  `data.voters`. Currently a poll can never be closed from the port. (Also fix
  `vote.rs` misusing the `poll.managePoll` string as the voter-facing subtitle.)
- `[ ]` **Position screen** (React `vote/PositionApp.tsx`). `vote/position` falls
  through to the generic `NodeApp`; it should compose content + candidate gallery
  + questions + poll list. Pulls in the next three items.
- `[ ]` **Candidate gallery + view** (React `vote/CandidateList.tsx`,
  `CandidateApp.tsx`). Image gallery of `vote/candidate` children (photo from
  `data.image`, per-user visibility), candidate opens as content with members
  hidden. Today candidates render as a plain node list.
- `[ ]` **Questions** (React `vote/QuestionList.tsx`, `AddQuestionButton.tsx`).
  Numbered `vote/question` list under a position with author chip, add-question
  (gated on `inserts`), and owner delete/sort. Absent. (Overlaps issue #138 —
  "replace questions with a comment model" — decide which model first.)
- `[ ]` **Amendments** (React `vote/AddChangeButton.tsx`). "New amendment" button
  (gated on `inserts` containing `vote/change`) that opens the add dialog with a
  redirect into the editor, pre-filling the title with the user's name under a
  position. `PolicyApp` lists existing amendments but can't create one.
- `[~]` **Poll-list affordances** (React `poll/PollList.tsx`). Poll rows under a
  policy/position show only a name link — add the vote-count badge, created-at,
  and owner delete.
- `[~]` **Voting-rights status** (React `vote/VoteApp.tsx`). No `canVote`
  permission check or "you (do not) have voting rights / you have (not) voted"
  card — eligibility is never surfaced. (Skip React's refresh-avatar; pull-to-
  refresh already covers it.)
- `[~]` Owner **Sort** entry buttons in the vote/comment/question lists (SortApp
  exists; just no link). Minor: block over-selection live on checkbox change
  (currently only errors on submit).

### Content & files — no file *creation* or file management

- `[ ]` **File upload** (React `util/FileUploader.tsx` via `nhost.storage.upload`).
  `nhost.rs` has no upload path at all, so `wiki/file` nodes can't be created and
  a document can't get a cover image. Needs a multipart upload + presigned-URL
  helper. **Biggest content gap.**
- `[~]` **Add-content dialog** (React `content/AddContentDialog.tsx`). `FolderAdd`
  hardcodes document/folder and ignores the node's `inserts` computed field
  (already queried, only used by comments). Drive the mime `<select>` from
  `inserts`, add the file-upload option and the free-text body for
  `vote/question`/`vote/comment`, and add a duplicate-name check.
- `[ ]` **Context creation + permission seeding** (React `AddContentDialog.tsx`
  `contextPerm`). Creating a group/event/context must insert the permission
  matrix; today a plain `insert_node` leaves a new context with no permissions.
- `[ ]` **File-node toolbar + sub-content** (React `file/FileApp.tsx` +
  `ContentToolbar`). A file page shows only the viewer — no delete / download-raw-
  file / edit / members / publish, no member chips, and no comments / changes /
  questions below it. Add the toolbar and a comment section (as documents have).
- `[ ]` **Raw-file download** (React `content/DownloadButton.tsx`) via a presigned
  URL, for every previewable file type (document `.odt` export already exists).
- `[~]` **Editor cover image + date** (React `content/Editor.tsx`). No image
  uploader/preview (writes `data.image`; ContentApp already *renders* it) and no
  `DatePicker` for `createdAt` (context-owner, niche). Save only writes
  content/name/mutable.
- `[~]` **Submit/publish confirmation** (React `content/PublishButton.tsx`).
  Submit fires directly with no confirm dialog / `submitWarning` for an
  irreversible action (the i18n keys exist, unused). Also no publish button on
  the read view.
- `[ ]` **`createdAt` subtitle** (React `content/ContentHeader.tsx`) — "created N
  ago" + absolute-time tooltip on content/file headers. Small.

### Members & invites — admin downgraded to hide-only + single email

- `[ ]` **Member administration** (React `member/MembersDataGrid.tsx`). Owners can
  only hide/unhide. Add promote/demote **owner**, toggle **active**, and **remove
  a member** (`delete_member` exists, no button). `MembersSetInput` needs
  `owner`/`active` (and `MemberFields` doesn't even query `active`).
- `[ ]` **Invite existing users by name** (React `invite/InvitesTextField.tsx`).
  Only single email invites work; add a `users` `displayName _ilike` autocomplete
  with multi-select (binding `nodeId` for known users).
- `[ ]` **Members entry point.** No owner "Members" button on a group/event view;
  `member` app is reachable only by typing `?app=member`.
- `[~]` **Bulk member import** (React `invite/InvitesFab.tsx` + `util/SheetReader.tsx`).
  Import a Fornavn/Efternavn/Email roster from `.xlsx` (on-conflict upsert).
  Heaviest lift — needs an xlsx parser in WASM; reasonable to defer.

### Mobile & navigation

- `[ ]` **Mobile app navigation** (React `layout/MobileMenu.tsx`). The app rail is
  desktop-only (≥1200px), so on a phone **Speak and Vote are unreachable** — the
  drawer only offers the context + Home. Events are attended on phones, so this is
  a real functional regression. (Distinct from the *styling* redesign #158, which
  was skipped — this is about reachability, not looks.)
- `[ ]` **Pending-invite badge** on the Home rail/nav item (React `useApps.ts`,
  `AppList`/`MobileMenu`). The invitations data already exists (`query_invitations`);
  surface a dot when there are unaccepted invites.
- `[~]` **Screen / projector launch** (React `useApps.ts`). On large unauth
  screens the rail offered a "Skærm" action opening `?app=screen` in a new tab;
  `ScreenApp` exists but nothing links to it.

### Auth & profile

- `[ ]` **Resend verification email** (React `auth/AuthForm.tsx`). On an
  unverified-user sign-in the port shows the error but never re-sends the email
  (`nhost.rs` has no `send_verification_email`), so a user who lost the first one
  is stuck.
- `[~]` **Public user profile** (React `layout/UserApp.tsx`). `wiki/user` nodes
  aren't routed (fall through to `NodeApp`); the port's `ProfileApp` is self-only
  and omits the **authored-content** list. Add: route `wiki/user` → profile, and
  show any user's memberships + events + authored content.

### Search & smaller UX

- `[~]` **Search** (React `layout/SearchField.tsx`). Add context-scoped results
  (filter by `contextId` inside a context), a **Ctrl-K** shortcut, **arrow-key +
  Enter** result navigation, and the parent-name secondary line on results.
- `[~]` **Speaker join types** (React `speak/avatars.tsx`). The port has 4 of 5
  types — the "misunderstood/announcement" type is dropped and procedure is
  renumbered `4`→`3`, which diverges from React's `order_by data desc` and any
  existing `speak/speak` rows with `data:"4"`. Restore type `4` or document/migrate.
- `[~]` Minor: post-login return-to-origin (`navigate(-1)` vs always Home);
  folder rail stays active during `?app=editor` and vote during `?app=poll`;
  per-app scroll-position persistence; sort save batched vs sequential.

## Open backlog (RadikalWiki GitHub issues still relevant)

- `[ ]` `#138` Replace "questions" with a comment model (see Voting → Questions).
- `[ ]` `#25` Content metadata attributes (e.g. a "keep longer" flag).
- `[ ]` `#69` Node revision / history table.
- `[ ]` `#115` Live collaborative editing.
- `[~]` `#128` Audio/MIDI — audio/video already preview natively; MIDI needs an
  external JS synth + soundfont (a CDN dependency).
- `[ ]` `#41` Export event participants (a new export beside the folder `.odt`).
- `[ ]` `#134` "Open" contexts anyone can join (backend + join UI).
- `[ ]` `#147` New permission system + editing UI (informs the perm app).
- `[ ]` `#145` Error / stacktrace reporting API.
- `[~]` `#33` PWA offline — installable (manifest/icon/SW registered); full offline
  needs the service worker served from the site **root** (`/sw.js` or
  `Service-Worker-Allowed: /`) — a deploy config, not app code (`src/pwa.rs`).
- `[~]` `#139` Native notifications — "your turn to speak" fires; poll-open
  notifications still to do (needs new-poll detection).
- `[blocked]` `#108` Drive icons/apps from mime data — the `mimes` table's `icon`
  is only a letter/number/questionmark avatar-mode hint (not a Material icon) and
  `MimeLoader` maps mime → a compile-time Rust component. Needs backend data first.
- **Need your input:** `#123` MimeAvatar-path-on-screen (unclear behaviour),
  `#155` nhost alternative (infra), `#135`/`#136` native DB primitives (backend),
  `#153` register campaign activity (large new feature — scope?).

## Known issues

- **Release build won't load in Servo** (`Module fetching failed`); only the debug
  build runs there. Loads fine in Chrome/Firefox. Uninvestigated, low priority.

## Intentionally excluded (React features that don't make sense to port)

- **ResultDataGrid, PollChart, PollChartSub** — commented-out / DevExpress-chart
  dead code in React. The port renders a live results table (`admin.rs`) + per-
  option `Bar` fractions instead — at or ahead of React.
- **Permission-editing UI** — `PermApp`/`PermList` are `null`/commented in React;
  the port's read-only `perm.rs` is already ahead. (Editing is issue #147.)
- **OldBrowser version gate** — a WASM/Dioxus build can't even start on the
  browsers it warned about, so the gate is moot.
- **HideOnScroll** auto-hiding bar, **RightDrawer** chrome, **SpeedDial** join FAB —
  heavy MUI interaction patterns; actions were relocated to the bar/headers and
  dioxus-primitives has no SpeedDial. No functionality lost.
- **"Current item" drawer jump** — dead code (`&& false`) in React.
- **Slate.js editor engine** — reimplemented over `contenteditable`/`execCommand`
  with Slate↔HTML round-tripping (functional parity; the only caveat is rich-paste
  fidelity vs React's Google-Docs paste mapping).

## Cross-cutting checks

- **GraphQL correctness:** filtered queries must omit unset fields (Hasura rejects
  `null` comparison expressions). Prefer `..Default::default()` +
  `skip_serializing_if`; grep `NodesBoolExp`/comparison structs when adding queries.
- **Permissions:** queries run with the user token; compare row visibility with
  `web/wiki` for the same account. Several audit gaps (candidate/question
  visibility, member admin) hinge on whether Hasura enforces the rule server-side.
- **Field naming:** cynic maps snake_case Rust → camelCase GraphQL
  (`mime_id` → `mimeId`); the schema is camelCase.

# AppView roadmap: the new backend

## Goal

`crates/appview` becomes the ONLY backend the Dioxus app talks to. NHost auth,
Hasura GraphQL, NHost storage, the Postgres schema and the serverless axum
sidecar (`backend/`) all retire at the big-bang cutover
(`docs/cutover-runbook.md`).

Done means all of these hold:

1. Every read, write and live update the frontend performs today
   (`src/graphql/*`, `src/nhost.rs`, `src/backend_api.rs`, `src/subscription.rs`)
   is served by the AppView, behind the `src/model.rs` seam, with no request
   leaving for NHost or Hasura.
2. Every request is authenticated by an AppView session bound to an atproto DID,
   and authorized by DID-keyed membership. No placeholder auth remains.
3. `cargo test --workspace` in `crates/` exits 0, and the browser suite
   (`just test-browser`) passes against an AppView instead of NHost.
4. The migration pipeline (dump, extract, load) carries every interim row the
   frontend can show, and the runbook's verification gates are green on staging.

The kickoff phase (`docs/rewrite-kickoff-plan.md`) is finished: the skeleton, the
generated schema, the Store seam, the OAuth stores and callback, the firehose
consumer, the durable ballot core, the loader and the deploy unit all exist. This
document is the phase after it: turning that skeleton into the backend.

## Where it stands (2026-09-19)

Built and tested in `crates/appview`, milestone by milestone below: sessions on
atproto OAuth, the read and write gates, the node tree the frontend routes by,
membership and the roster, meetings (speaker lists, the projector, the canvas),
voting with blind-signed secret ballots, live topics, files, every endpoint the
interim sidecar served, search and the feeds, and `appview import`, which loads
a migrated wiki. `docs/appview-api-coverage.md` sets every data call the
frontend makes against the method that answers it, and none is left without.

The frontend runs on it. Built with the `appview` feature, the three modules
that speak to the interim's backend (`graphql`, `backend_api`, `nhost`) are the
AppView's under the same names (`src/appview/`), so no component changes which
function it calls; built without it, which is what ships, nothing is different.
`scripts/test-browser-appview.nu` builds it, starts a dev AppView, and drives
it in headless Firefox: the screens draw what was seeded, writing through them
lands (a comment, a page through the editor, a place in the speaker list, a
cell on the canvas, an open ballot and a secret one, blinded and cast from the
browser's own wasm), and a change made elsewhere arrives without a reload.

Not built: mail to an invited address (M4), and the parts of voting that wait
on a decision that is the owner's to make (M6). What needs a person and cannot
be tested from here: one real browser login, and with it the token exchange,
the profile read and posting to a PDS. The interim's own browser suite
(`test-browser.nu`) is written against the interim's backend and an account on
it, and has not been ported; the AppView's is the smaller one named above.

## Findings that shape the plan

- **The workspace did not build in the devshell.** `atrium-oauth`'s default HTTP
  client pulls `reqwest/default-tls`, so OpenSSL, against the recorded
  rustls-only decision (`docs/atproto-stack-decisions.md`, Rust server
  libraries). Fixed in M0 by supplying the one rustls client the decision asks
  for.
- **Writes trusted the caller.** `xrpc::caller_did` read the DID straight out of
  the `Authorization` header, so anyone could write as anyone by naming them.
  Replaced in M1 by sessions.
- **`/login` is an SSRF primitive unless guarded.** It is unauthenticated and
  fetches whatever host the caller names. The outbound client now refuses
  non-https and non-public addresses, at resolution time, so DNS rebinding gains
  nothing. A dev instance (no public URL) stays unrestricted for a local PDS.
- **The engine was five minor versions behind, and it mattered.** turso 0.2.2
  had no `EXISTS`, no `IN (subquery)`, no upsert, and ignored the foreign-key
  pragma, so nothing was referentially checked. 0.7.2 has all four. Turning
  enforcement on surfaced two real defects at once (the firehose and the
  loader, both fixed). Recursive CTEs are still missing, which M3's path
  resolution has to design around.
- **The extractor leaves gaps the loader will now refuse.** A context's
  `parent_id` is carried over verbatim, but in the interim tree a group or event
  can sit under a folder, which is a `document` here, so that key dangles. The
  node that owns a context without holding a membership row in it (a real case:
  `docs/read-permissions.md`) gets no owner row. Interim public contexts are all
  extracted as private. Extracted polls are never loaded, and are extracted
  wrong: the question is read from `data.question` and the state from
  `data.open`, neither of which the interim writes (a poll's name is its node's
  name, and open is `mutable`), and `minVote`, `maxVote`, `hidden`, the place in
  the tree and the RESULT are not read at all. Ballots cannot be carried; the
  outcome of every past vote can, and `poll.counts` is where it goes. M3 and M9,
  where all of it is now carried.
- **Nobody would have found their own account.** The extractor carries an
  interim account under its interim id, "until the DID binding runs", and no
  such binding existed. Everyone with an account, which is everyone who has
  ever signed in, would have come back after the cutover as a stranger: their seats,
  owner rights and authorship held by an account that can never sign in, and
  not even open to a new invitation. A person now takes their old account over
  by signing in with the address it was registered under, where the interim
  had verified that address and a trusted PDS confirms it; anyone else is
  handed a seat at a time by claim link (`crates/appview/src/legacy.rs`).
- **A claim link the interim had spent would have opened its seat again.** The
  interim keeps the token on the row after the seat is taken. Once a seat held
  by a carried account can be claimed, that old link, sitting in somebody's
  inbox, claims it. Spent tokens are left behind, and an owner mints a new one
  for a seat that needs it.
- **What was deleted would have come back.** The interim bins a comment by
  stamping it, and the extractor took no notice of the stamp, so every comment
  somebody had deleted was extracted as live. A comment now comes across in the
  bin it was in. A reaction or a report in the bin is left behind.
- **The first audit of the API read the data layer and not its callers.** The
  frontend deletes a comment through the same `bin_node` and `update_node` it
  uses for a page, from inside the comments component, so the table had a
  comment as something one posts and reads. It can also show a picture, be
  deleted, be emptied in place when it has been answered (deleting it would
  take everyone who answered along), and come back from the bin. None of that
  existed here, and the extractor dropped the picture without a word. Building
  it found three more: a move to another group left every ANSWER in the old
  group, read there and not in the new; a purge deleted the comments on a page
  and left the answers to them, and every reaction, in the database for good;
  and an answer was news of nothing, so the feed left answers out and the list
  of what has gone astray listed every one of them. A comment now knows the
  document its thread is on (`root_id`), which is what all three go by.
- **A row shown before the server has it never went away.** The components
  show a comment or a speaker at once under a key they choose, and drop that row
  when a fetched one carries the same key. The AppView names rows itself, so in
  a real browser a posted comment sat as "Sending…" beside its own copy for as
  long as the page was open. No test below a browser could see it: the data
  layer's answers were right. The layer now hands a row's key back as its
  component chose it (`src/appview/seen.rs`).
- **Saving a page re-dated it to midnight.** The editor sends a page's day back
  with every save an owner makes, and a day alone is stored as its midnight, so
  a page made a minute ago read "19 hours ago" after its first save. The interim
  does the same. Here the day a node already has leaves its time of day alone.
  Seen in the browser run, where a page is made and saved as a person would.
- **Looking a ballot up named the voter.** `getBoardEntry` asked that the
  caller be able to read the poll, so in a closed group a voter could only check
  their ballot with their session on the request: their name and their token in
  one call, which is the pairing the blind signature exists to keep from the
  server. It takes no session now, as a cast does.
- **A lost reply would have cost a vote.** The AppView signs a voter's tokens
  once, and again only for the same blinded tokens. A browser that blinded
  fresh ones after a dropped answer would have been refused for good. The
  frontend keeps what it blinded before it asks, and a retry that finds its
  token already spent asks the board what that ballot said instead of assuming.
- **A change said where, and a feed needs to know what.** A change named the
  page a comment was on and not the comment, so a feed hearing of one had
  nothing to fetch. A change now also carries the row it made.
- **The clock came off the JWT.** Countdowns and cooldowns are reckoned against
  rows the server stamped, and the interim read the difference between the two
  clocks out of its token. A session here is no JWT, so every answer carries
  the server's clock (`x-server-time`) and the client measures the difference.
- **A report's screenshot was for nobody.** It is filed in a context of its
  sender's, which whoever reads the reports is usually no member of. They may
  open it now, and a report takes only a screenshot its sender uploaded.
- **The site had no home.** The interim's root is a context like any other: its
  members run the site, its content is the welcome page, and what sits at the
  top of the tree is made in it. The extractor skipped it as "the root every
  path starts under", and the AppView had nothing in its place. After a cutover
  nobody could have started a group at the top level, since a context is made
  under one the caller owns and at the top there was none to own; nobody could
  have read a report or put right what had gone astray, which were held to
  "whoever owns a site", and that meant any blog; and the welcome was gone. The
  home is a row now (`kind = 'home'`, the empty path), carried with its owners
  and its welcome, and made at start where a datastore has none.
- **A place said nothing about itself.** A group's front page, its cover and a
  redirect are the context's own `data`, as a page's are. `context` had no
  column for any of it and the extractor dropped it without a line in the
  report: every group's front page was to be lost. The public list also left
  out every site, where the interim leaves out the root.
- **Nothing could run a load.** The loader was a library with no binary, and
  what a poll came to, a canvas and the reports live in tables it cannot know.
  `appview import` is the load step: one transaction, the same twice, and
  refused on a datastore somebody has signed in to.
- **There are no schema migrations.** The entity tables are plain
  `CREATE TABLE`, so a datastore file made by an older binary keeps its old
  columns. Until migrations exist the file records its schema version and a
  binary refuses to start on another one, rather than failing a query at a
  time. Pre-cutover that costs nothing, since the view is rebuilt from the
  migration pipeline; after it, migrations become real work (M9).
- **The interim serves every member's email to every member.** A column
  permission is per role and an owner is role `user` too, so one plain member
  could read 1,467 of the organisation's 2,007 addresses. The AppView can draw
  the line Hasura could not: `listMembers` serves an address to an owner of
  that context and to nobody else, and matches a search against one for an
  owner only.
- **Reads were ungated.** Every read served private content to anyone. Gated in
  M2 on visibility and membership.
- **Reactions stayed ungated until an audit of the whole API found them.** The
  kickoff had classed reactions as mirrors of public records, so anyone signed
  in could react to any string with any string, and who had reacted to a closed
  group's comment was served to the signed out. The interim's reactions are a
  member's, on content and comments. They are now held to that.
- **`active` is voting rights, not a read gate.** The interim reads by
  membership alone, and `migrations/0024` records what reinterpreting a write
  model as a read model cost: a context went dark for its own members. The
  AppView's gate follows the same line.
- **A document could not be addressed by path.** The entity schema gave
  `context` a slug and `document` none, while the frontend routes every node by
  its key path (`/a/b/c`), so every existing link would have broken at cutover.
  The extractor dropped the key without a word, and with it `index`, `mutable`,
  `attachable`, the owner, `updatedAt` and the bin; it REPORTED a file's
  `fileId` and `type` as gaps rather than carrying them, which would have
  migrated every attachment as an empty page; and `context.kind` rejected
  `wiki/site`. Reconciled in M3.
- **Two people writing at once, and one of them failed.** The engine's default
  is to refuse a write the moment another connection holds the write lock, and
  every request here has its own connection. Measured with 16 writers: 371 of
  400 transactions refused with "database is locked". A meeting is exactly
  that load (a room joining a speaker list, 500 ballots in a minute). Every
  connection now waits its turn, up to ten seconds.
- **Speaker lists were classed as ephemeral and dropped by the extractor**, but
  the speak app persists them and an assembly reads them back. M5 gives them
  tables; M9 migrates them or records why not.

## Milestones

Each lands with tests, committed on its own. A box is ticked only when the code
is merged and its tests pass.

### M0: build hygiene

- [x] One rustls `reqwest` client for every outbound call; OpenSSL out of the
  dependency graph; `cargo test --workspace` green in the devshell.
- [x] turso 0.2.2 to 0.7.2; foreign keys enforced and verified on every
  connection; the two-step upserts collapsed into real ones.
- [x] Concurrent writers wait for the write lock instead of failing.

### M1: identity and sessions (replaces NHost auth)

- [x] `session` table; opaque bearer tokens, stored hashed, with expiry.
- [x] `GET /login` (handle to authorization URL), bound to the browser by a
  cookie against login CSRF; `/callback` returns a one-time code to an
  allow-listed frontend origin, and `createSession` redeems it, so no credential
  is ever in a URL.
- [x] `getSession` and `deleteSession`; `Caller` and `MaybeCaller` extractors;
  the placeholder `caller_did` deleted.
- [x] CORS for the configured frontend origins.
- [x] Production client metadata (`/client-metadata.json`) when a public URL is
  configured; the loopback profile otherwise.
- [x] Outbound requests confined to public https addresses.
- [x] Profile hydration on login (`crates/appview/src/profile.rs`): the handle,
  display name and avatar, read from the member's own PDS beside the login and
  never in its way. PDS-agnostic: `getSession` and the profile record, not
  Bluesky's AppView. Like posting, it has not met a real PDS yet.
- [ ] A confidential client (`private_key_jwt` and a served JWKS). Not needed
  until the publish seam writes to members' repos and wants long-lived PDS
  tokens; the AppView's own sessions do not depend on them.
- [ ] One full login in a real browser against a real PDS. Everything around
  the token exchange is covered; the exchange itself needs a human.

### M2: authorization core

- [x] DID-keyed `is_member`, `is_active_member` and `is_active_owner`, one module
  (the predicates the kickoff plan deferred). Membership reads and writes;
  `active` is voting rights and gates only what the interim predicates gated.
- [x] Reads gated in SQL, before any `LIMIT`: public, or member, or author. A row
  the caller may not read is indistinguishable from a missing one.
- [x] Writes gated: membership to create or comment; a comment's context comes
  from its subject, a document's parent must be in its context.
- [x] Invite binding by claim token (`claimMembership`), and the owner's claim
  link.
- [x] Invite binding by email. A login asks for `transition:email`, and an
  invitation sent to an address the account's PDS says is CONFIRMED is handed
  to whoever signs in with it. Only a configured PDS is believed
  (`APPVIEW_TRUSTED_EMAIL_PDS`, by default Bluesky's own hosts): anyone can run
  a PDS, and one that lies about an address would walk its owner into that
  address's invitations, voting rights included. Everyone else uses a claim
  link.
- [x] Authorship or ownership to change a node (`Standing`, with
  `updateDocument` in M3).

### M3: the node tree the frontend routes by

The frontend knows one tree of typed nodes, each reachable by the path of keys
in its URL; the store keeps typed entities in separate tables. The design that
joins them:

- `context` and `document` are the two spines of the tree. Both carry `slug`
  (the interim `key`), a stored `path`, `idx`, `attachable`, `owner_did`,
  `updated_at` and `deleted_at`; `document` also `mutable` and `data`. A parent
  may be either kind (a group can sit in a folder), so `parent_id` is a plain
  column on both and the write path keeps it honest.
- `path` is STORED and maintained on write, as the interim does with a trigger.
  turso has no recursive CTEs, and a stored path makes resolution one indexed
  lookup at any depth. A rename or a move rewrites the subtree's prefix. It is
  unique among live rows only, so a binned node does not hold its URL hostage.
- The server picks the slug (`name`, then `name-2`, ...), because only it can
  see every sibling; the frontend finds collisions today by attempting inserts.
- Kinds that are nodes in the frontend but have their own state and a URL of
  their own (polls, canvases) become `document` kinds with a side table keyed by
  the same id, when their milestone arrives. Speaker lists and the projector
  have no URL, so they are plain tables beside the tree (M5).
- Who may create what under what is the interim's per-context template
  (`context_permission_objects`), as one static table: (kind, role, parents).
  Reading stays by membership (M2); this is the write model only.

The steps:

- [x] Schema reconcile: slug, stored path, idx, attachable, owner, updated_at and
  deleted_at on both spines; mutable and data on documents; the `site` kind.
  Generated DDL on both engines, and carried by the dump, extractor and loader.
- [x] Path resolution across contexts and documents, in one lookup.
- [x] The server picks a new document's slug, in one write transaction, across
  both tables' namespace.
- [x] `getNode`: a node by path or id with its children of both kinds in one
  ordered list, a crumb per segment (nameless where the caller may not read
  it), and the viewer's standing. One call where the interim makes several.
- [x] The write model: which kind a role may create under which parent, and the
  folder lock with its owner and discussion exemptions. One static table,
  carried over from the interim's per-context template. (A closed poll refusing
  votes belongs to M6.)
- [x] The ordinal that letters submitted motions (the interim's `get_index`,
  by its rule: submitted nodes only, by index, then last update, then id), on
  the page and in the listing alike; the profile behind every DID a page names.
  It inherits the interim's weakness: an edit bumps `updated_at`, so correcting
  a submitted motion can change its letter unless the chair has set the order.
- [x] The parent reference on a node ("in <parent>"), for the feed rows that
  quote what they are about: every feed row and search hit carries it, when the
  caller may read the parent too.
- [x] Groups and events (`crates/appview/src/context.rs`): made, renamed,
  opened to the public, locked, binned and restored. This was missing from the
  plan altogether: a context could only arrive by migration or off the
  firehose, so no group could have been started after the cutover. Making one
  is a single transaction where the interim needs four writes from the
  browser, and needs no permission template: the one rule every context had is
  `crates/appview/src/authz.rs`.
- [x] Update, with the interim's edit rule (an author edits a draft, an owner of
  the context edits and arranges anything); reorder and the lock with it.
- [x] The bin: soft delete of a subtree, restore of exactly what went together,
  the listing.
- [x] Move: a subtree to a new parent, every stored path in it rewritten, the
  binned ones too so a later restore lands where its parent now is.
- [x] Copy and purge (`crates/appview/src/tree.rs`), and moving between
  contexts. All three mind what a node points at, because a comment, a file and
  a poll are each read through their OWN context: a move takes them along, or a
  file would stay readable by the old group and not by the new; a copy gets
  file rows of its own over the same bytes; a purge takes the files nothing
  else points at. A purge refuses a bin entry that holds a poll, and a running
  poll does not change groups.
- [x] The home, and what a place says about itself
  (`crates/appview/src/context.rs`). `getNode` of the empty path is the home;
  `createContext` makes what sits at the top under it, a site included;
  `updateContext` takes a place's content and its data, which a read of the
  place serves and a search finds. `APPVIEW_SITE_OWNER` seats a DID as an owner
  of the home at every start: the operator's way in, to a new site or to a
  loaded one none of whose owners can sign in.
- [x] Other people's drafts are theirs alone. The interim keeps an unsubmitted
  motion, amendment or candidacy out of every listing but its author's, by a
  filter the FRONTEND adds to its queries, so a backend that answers the same
  questions without the filter lists everybody's drafts to everybody. It is
  the server's rule here (`authz::listed_document`), over a node's children,
  their counts and `listChildren`. It is not a read gate: a draft still opens
  by its address. And it names the kinds that have a submit step, because on
  anything else `mutable` means something else, which is how the interim once
  blanked a canvas for all but one person.
- [x] What the second pass over the frontend's callers turned up beside that:
  `purgeOrphan`, since the view that lists what has gone astray exists to clear
  it away and the AppView could only list; and an owner redating a page or a
  meeting (`created_at` on `updateDocument` and `updateContext`), which is how
  minutes come to be filed under the day of the meeting.
- [x] A comment's whole life (`crates/appview/src/comment.rs`): posted with a
  picture of its author's from the same context; deleted by its author or an
  owner, which empties it where it stands if it has been answered and bins it
  otherwise; listed in the context's bin, restored from it, purged from it
  with its reactions and its picture.
- [x] Search, the recent feed, contributions, orphans
  (`crates/appview/src/search.rs`, `feed.rs`). Search runs over an index of each
  document's WORDS, lowercased by Unicode's rules: over the stored Slate JSON
  every document matched `children` and `type`, and SQLite folds case for ASCII
  only, so `årsmøde` did not find `Årsmøde`. The index is derived, rebuilt at
  every start and kept fresh by writes, so the loader need not know of it. A
  title match is ranked before the cut, not after. The feed is the interim's
  predicate (submitted content, comments and reactions, where the caller
  belongs) over light rows; the interim's rows each carried their whole
  document. A row carries what the frontend draws one from: how the page
  begins, the picture that goes with it, and for an answer or a reaction the
  comment it is to.

### M4: membership and roster

- [x] What a member row holds: the roster's name (the only label 83 percent of
  rows have, and which the extractor dropped), `hidden` and `accepted`. Whoever
  made a context is realized as an owner of it, or the general secretary loses
  Landsmøde 2026 at cutover. A member row on something that is no context is
  reported, not loaded.
- [x] Paged, filtered member list, for members only, with addresses and hidden
  rows for owners only; the voter count.
- [x] Invite by address, by name and by account, as one invitation or a whole
  roster, skipping whoever is already there; update and remove, with a context
  always keeping an owner who can sign in.
- [x] The caller's invitations: list, accept, and decline or leave by removing
  oneself; claim links for every roster row.
- [ ] Reaching an invited address. Nothing sends mail, so an owner hands out
  claim links by hand. Since M2 binds by the account's confirmed address, that
  is only needed for members whose PDS is not one configured as trusted, or
  whose account has another address than the roster's.
- [x] Author chips (`setDocumentAuthors`): accounts, names with no account, or a
  group. The interim lets a group be named as an author, and the extractor read
  every chip that pointed at a node as a person, so a branch that put a motion
  forward was migrated as an account with the group's id.

### M5: meetings

- [x] Speaker lists: several per context, a queue served by the chair's
  override, then the kind of contribution, then arrival; a second tap takes no
  second place; a per-turn clock anchored on the server's time.
- [x] Projector state: one row per context (the node on screen, a focus anchor
  that does not follow the screen to another node, comments and feed toggles),
  where the interim packs it into relation names.
- [x] The canvas (`crates/appview/src/canvas.rs`): a board a room paints
  together, one cell per person per cooldown. It was not in the plan: it came
  to light when every data call the frontend makes was gone through. The
  interim keeps each cell as a node and the cooldown in a trigger. Here cells
  have a table, the cooldown is checked under the write lock, and a listener
  reads what was painted since its last answer rather than the board again.
  The boards that have been painted come across with their cells (M9).

Speaker lists and the projector are not migrated, on purpose: a queue or a
screen from a meeting that has ended is of no use to the next one, and a list
is one click to make again.

### M6: voting

A poll is a node (a document of kind `poll`, so it has a URL, moves with its
motion and goes to the bin with it) and a `poll` row of the same id
(`crates/appview/src/poll.rs`, over `crates/ballot-store`).

- [x] Open and close a poll; eligibility freeze; per-poll issuer key. The roster
  is whoever holds voting rights in the context when the poll opens. The issuer
  key is kept sealed under `APPVIEW_SECRET` while the poll is open, so a copy
  of the database forges nothing, and is destroyed at close. Closing seals the
  board under the write lock a cast takes, so no ballot lands after the count.
- [x] Blind token issuance; cast to the board; tally; status. Issuance is signed
  in and records only THAT a voter was served: no token, no time, no order (a
  random rowid and the poll's opening time), since either would line up with
  board positions. A cast carries no session. A lost reply can be asked for
  again, with the same blinded tokens, and mints nothing new.
- [x] Open (non-secret) polls: one named ballot per voter, at their frozen weight.
- [x] A board for every poll in one store. `ballot-store` held a single poll.
- [ ] Publishing the board as atproto records, signed inclusion receipts, and a
  signed close-out digest. All three wait on the owner's custody call
  (`docs/ballot-board-custody.md`). Until then the board is served from here:
  to whoever may see the counts, in token order (board order is the order the
  room voted in), and one entry at a time to a voter who knows their token.
- [ ] Delegation. The roster resolves it (`freeze_at_open`) and issuance and
  counting honour the weights, but nothing writes a delegation: the interim has
  none, and what signs an assignment is undecided.

Decided here without the owner, and cheap to change now:

- **A hidden tally stays hidden after the close**, for everyone but the owners
  of the context, and the board with it. That is the interim's rule, and an
  election is where it is used: who won is announced, not by how much. The cost
  is that only the owners can recount such a poll. A voter can still check that
  their own ballot is there and says what they said.
- **Nobody joins a poll that is already open.** Someone given voting rights
  after it opened votes in the next one. The interim checked at the moment of
  casting.
- **A poll with nobody to vote in it does not open** (`NoVoters`), which is
  what a chair sees who forgot to hand out voting rights.

### M7: live updates

- [x] `/ws` topic protocol (`crates/appview/src/live.rs`): a listener
  authenticates with its first frame, subscribes to `context:<id>`,
  `user:<did>` or `public`, and is granted a topic only if it may read it. A
  change names what changed and never what it changed to, so a client refetches
  through the gated reads. The relay it replaces sent every delta to every
  connection, unauthenticated.
- [x] Every write path built so far publishes its change.
- [x] Re-checking a standing subscription when its listener's membership ends.
  Every change to a context's membership has each of its listeners checked
  again; one who may no longer read it is told so once
  (`{"op":"unsub",...,"revoked":true}`) and hears nothing of it after.

### M8: blobs and the carried-over endpoints

- [x] Upload and download with read authorization; signed links for third-party
  viewers (`crates/appview/src/blob.rs`). A file belongs to a context and is
  read by whoever may read that context. The bytes sit on disk under their
  SHA-256, streamed both ways, with ranges for a player. Only types that cannot
  run are shown in place; a page, an SVG or any XML is a download.
- [x] A ceiling on what one member and what one context may store, beside the
  cap on a single file (`StorageFull`). The interim has neither. Deleting gives
  the room back.
- [x] Copying the interim's files across, each under its old storage id so that
  nothing that points at one is rewritten. `scripts/dump-interim-files.nu`
  downloads them (read-only, the owner's to run; proven against a stand-in for
  NHost's storage, since the real one is not ours to reach), and `appview
  import-files` files each under the context of what points at it: a page's
  file and cover, a place's cover, a comment's picture, a report's screenshot.
  One that came across short is refused by its size; one that nothing points
  at is listed and left.

#### The interim sidecar's other endpoints

Carried over one at a time. Handle typeahead is not among them: the browser
asks Bluesky's public API itself. The steps:

- [x] Roster parsing (`parseRoster`): the office's .xlsx into the rows
  `inviteMembers` takes.
- [x] The log proxy (`POST /log`), with symbolication of the wasm frames passing
  through. It takes no session, so what it remembers about builds is bounded:
  made-up build hashes cannot push a real build's symbols out or grow without
  limit.
- [x] Push: subscribe, unsubscribe, notify, reply (`crates/appview/src/push.rs`).
  The encryption and VAPID are the interim's, against the RFC 8291 vector. A
  subscription is a device's and is keyed by DID, where the interim keyed by
  email. A push endpoint is a URL a client hands over, so it goes out through
  the guarded client, which also closes the DNS-rebinding hole the interim
  accepted. A notification may link only into the app. The cutover must carry
  the interim's VAPID key pair over: the frontend has the public half compiled
  in, and every browser's subscription is bound to it.
- [x] Feedback and crash reports (`crates/appview/src/feedback.rs`). The interim
  files them as nodes under the root node; here they have a table. A crash seen
  again is one row with a count and its people, under the interim's digest, so
  a crash known before the cutover keeps its row once migrated. Reports need no
  session, so those without one share a budget of ten a minute. The interim's
  reports come across, a crash it filed twice as one row (M9).
- [x] Metafile rendering (`renderMetafile`): EMF and WMF figures in Word and
  PowerPoint files to SVG, or PNG where the SVG emitter declines. The PNG path
  draws text with whatever generic sans the host has, so a host with no fonts
  installed renders those figures without their text.
- [x] Posting to a member's own PDS (`shareToBluesky`), through the OAuth
  session atrium keeps from their login, which replaces the interim's own
  sealed copy and hand-rolled refresh. What is refused and what record is
  built are tested; the write itself has not met a real PDS, and cannot before
  the human login listed under M1.

### M9: frontend swap, migration, cutover rehearsal

- [x] Every data call the frontend makes, set against the method that replaces
  it (`docs/appview-api-coverage.md`). Going through them found four things the
  plan had missed: no way to start a group, reactions nobody gated, the canvas,
  and the people pickers. All four are built; nothing in the table is left
  without an answer.
- [x] A typed client for the AppView (`crates/appview-client`), generated from
  the lexicons by `crates/lexgen`, and contract tests that drive the real
  router with it over HTTP, refusing any field no lexicon names. All 83
  methods are called; a lexicon added without a call fails the tests. Writing
  it found what the frontend would have tripped over: `createDocument` and
  `createCanvas` answered with an id and not with where the new node is, which
  a screen that has just made a page navigates by; `listChildren` had no light
  rows of both kinds for the drawer to expand by; a reaction was stamped to the
  second and so sorted ahead of the comment it was to; and clearing the
  projector took a `null` that a typed client had no way to say.
- [x] An AppView client behind `src/model.rs`, standing in for `src/graphql/*`,
  `src/backend_api.rs` and `src/nhost.rs` (`src/appview/`). The components call
  the data layer as `graphql::*` and read `model::*`, so this is a second
  implementation of that surface, switched in by the `appview` cargo feature
  under the names every caller already uses. The default build, which is what
  `main` ships, is untouched. The components write through generic calls,
  since to the interim everything is one table's row, so a write asks what kind
  of thing its id names (`seen.rs`: every read says what it saw) and goes to
  the method that belongs to it. A live query is "refetch when anything
  matching this changes", which is what a `/ws` topic is, so `Wire` and its
  builders keep their names and say which context to listen to and which
  changes there are the view's own (`wire.rs`, `watch.rs`, `hub.rs`). Signing
  in is the one part that cannot hide behind the seam: a password form became
  a handle and a redirect (`components/auth_atproto.rs`), and a session lasts a
  month with nothing to refresh. A secret ballot is blinded, signed and cast
  from the browser (`ballot.rs`). Tested three ways: `live.rs` starts
  `crates/appview-dev` and asks a running AppView what the components ask,
  through the functions they call, over a real socket where that matters
  (`cargo test --features appview`); both builds pass the wasm clippy gate; and
  `scripts/test-browser-appview.nu` drives the built app in headless Firefox.
  Like the interim's layer it asks for a node once when two views want it at
  the same moment (a page and its crumbs, the drawer and the search box), and
  remembers nothing once the answer lands. Left as it was: the home page's two
  hints about which email to register with are the interim's words: here an invitation by address finds a person through the
  address their provider confirms, else by claim link, which is for whoever
  writes the site's copy to say.
- [x] The extractor and the load cover every kind the interim holds: the tree,
  members, comments, reactions, what each poll came to, canvases with their
  cells, reports, which contexts are open to everyone, and the address each
  account is recognized by. A comment comes with its picture, emptied if it
  was, and in the bin if it was. What is left behind on purpose (a deleted
  reaction, a spent claim link, a speaker list, an address nobody verified) is
  counted in the report under `left_behind`, so none of it is silent.
- [x] `appview import <extraction.json>`, the load step as a command, proven
  from an interim-shaped snapshot through the real extractor to the AppView's
  own reads.
- [x] A person takes their interim account over at sign-in, or a seat at a time
  by claim link (`crates/appview/src/legacy.rs`). A test reads the schema, so a
  table that names an account and is not handed over fails the tests.
- [x] The load as the deployment runs it: `wiki-appview-import`, a one-shot unit
  in the NixOS module that shares the service's private state directory and
  confinement, takes the extraction as a systemd credential and the files as a
  read-only bind, and stops the service while it loads. The VM test loads a
  made-up wiki this way and reads it back over HTTP.
- [ ] The field-gap report empty on a real dump, which is the owner's to take.
- [x] A browser run against the AppView (`just test-browser-appview`): small,
  and its own, since the interim's suite is written against the interim's
  backend and an account on it.
- [ ] Staging rehearsal of the runbook, and the interim's browser suite ported
  to run against an AppView.

## Working rules

- `cargo test --workspace` in `crates/` is the check. There is no CI job for it
  (`crates/README.md` says why), so run it before every commit.
- Anything that stands in for real behaviour says so in its doc comment and is
  listed under "Findings" until it is replaced.
- The frontend is not touched before M9 except to add to the seam, so the live
  app keeps shipping from `main` throughout.

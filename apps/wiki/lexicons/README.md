# wiki.radikal.* lexicons

atproto Lexicon schemas for the app, drafted for the atproto rewrite (pre-rewrite plan #7). They come
in two categories:

- **Record lexicons** define the wire format of records the app publishes to atproto repos so other
  AppViews can read them. They are the federation-boundary contract only; the private,
  org-authoritative half (ballots, roster, delegation, eligibility, internal deliberation) is owned by
  Rust serde types in the backend and never becomes a record.
- **Method lexicons** (`query` / `procedure`) define the AppView's own XRPC API: the read/write
  methods the AppView serves at `/xrpc/{nsid}` over its canonical DOMAIN entities. These are NOT
  published records; they are the contract the frontend seam will consume. See the Methods section.

See `docs/atproto-stack-decisions.md` (Lexicon-to-atrium codegen pipeline) and
`docs/atproto-domain-model.md`.

A third category is neither public nor private to the backend: records held in **atproto spaces**, one
space per context, readable by its members (`docs/atproto-spaces-redesign.md`). `context` is the space's
type, `contextProfile` what a context is, `node` anything with a place in its tree (one record type with
a `kind`, as the backend has one table), and `comment` and `reaction` serve in a space as they do in
public. `spaceDefs` has what they share. Inside a space a body stays the editor's own JSON, named by
`contentFormat`, so `document` is no longer excluded there. `crates/wiki-records` has them as Rust types,
held to these files by a test, with the mapping to the backend's rows.

## Scope

Only entities that are meaningfully and safely publishable get a lexicon:

- `wiki.radikal.post`: a member's feed post (the social unit).
- `wiki.radikal.statement`: a member's personal public statement.
- `wiki.radikal.resolution`: the org's published outcome of a motion/election.
- `wiki.radikal.comment`: a public comment on a public item.
- `wiki.radikal.reaction`: a member's emoji reaction to a public item
  (comment/post/resolution), addressed by a strongRef. One record per (reactor,
  subject, emoji); deleting the record removes the reaction (toggle). Net-new (the
  old wiki had no reactions), so it maps no legacy mime.
- `wiki.radikal.group` / `wiki.radikal.event`: the opt-in-public container contexts. The
  group/event kind split is carried by the two NSIDs; a record exists only while the context is public.
- `wiki.radikal.poll` / `wiki.radikal.ballotEntry`: the public poll announcement and the
  anonymized bulletin-board entry (repo custody pending an owner call).
- `wiki.radikal.document` is EXCLUDED for now: documents store Slate JSON internally, and the
  public rich-text representation (what a document record's body looks like on the wire) is a
  rewrite-time decision that has not been made yet. No lexicon until it is.

Always-private entities (`voted`, roster/eligibility/delegation, membership-as-affiliation,
projector/speaker) deliberately have NO lexicon. The ballot is SPLIT, not simply private: the
org-side ballot row (eligibility, token issuance, resolved weights) is always-private and has no
lexicon, while the public ANONYMIZED board entry (token + choices, no voter identity) is exactly
what `wiki.radikal.ballotEntry` describes.

## Methods (the AppView's XRPC serving layer)

These describe the AppView's own read/write API (`crates/appview/src/xrpc.rs`), served at
`/xrpc/{nsid}`. They return the AppView's canonical DOMAIN entities (the reconciled internal shapes),
NOT the published repo records above; `wiki.radikal.defs` holds the shared view objects
(`documentView`, `contextView`, `commentView`, `reactionView`, `authorView`, `memberView`,
`userView`, `blobView`, `pollView`, `boardEntryView`) they reference. This is
why a `documentView` exists even though the `document` RECORD is excluded: the served entity shape is
settled, but its public rich-text record shape is not.

Queries (GET). Every read serves only what the caller may read (`crates/appview/src/authz.rs`): a
row is readable when it or its context is public, when the caller is a member of its context, or when
the caller wrote it. A row the caller may not read answers exactly as a missing one does.
`getReactions` answers what the caller may not read with an empty list rather than an error.

- `getNode` is what a screen loads: a node by path or id with its children of either kind, its
  breadcrumbs, the profile behind every DID it names, the letter a submitted motion carries, and
  what the caller may do there.
- `getProfile` is who a DID is, and `searchPeople` finds someone to invite or to credit, with the
  groups by that name too, since a group can be an author. Both are for the signed in.
- `listMembers` is a context's roster, for its members only, with addresses for its owners only;
  `getVoterCount` is the number a poll's turnout is out of.
- `getDocument` / `getContext` return a single entity; `resolveNode` returns the context or document
  a path names. Every node stores its path, so that is one lookup at any depth.
- `search` finds documents and contexts by what they are called and what they say; `listRecent` is
  the feed; `listContributions` is what a person or a group has put forward; `listOrphans` (the site
  owner's) is what has lost its parent, and `purgeOrphan` clears one of those away. Their rows are light: a name, a path and what the row is
  about, never a document's content.
- `listChildren`, `listContexts`, `listRecent`, `search`, `getComments`, `getReactions` return an
  object wrapping a named array (`{ documents: [...] }`, `{ contexts: [...] }`, ...). Lists are
  wrapped, never a bare top-level array, because a bare array is not a valid lexicon `output.schema`
  and the wrapper leaves room for a future `cursor`.

The session (`crates/appview/src/session.rs`): a login is `GET /login?handle=&return=`, which sends
the browser through the member's own PDS and back to `return#code=<one-time code>`; `createSession`
redeems that code for an opaque bearer token. `getSession` returns the caller as a `userView`, and
`deleteSession` signs out the presented session and no other. `/login` and `/callback` are browser
navigations, not XRPC, so they have no lexicon.

Procedures (POST, authenticated; the caller's DID comes from the session, never the body):

- `createContext` makes a group or an event, or directly under the home a site, with the caller as
  its first owner; `updateContext` renames it, opens it to the public, locks it, or changes what it
  says about itself (`content`, `data`); `deleteContext` and `restoreContext` are its way in and out
  of the bin. Everything is under the home, the one context whose path is the empty one: its owners
  run the site, so what sits at the top is theirs to start, and the reports are theirs to read.
- `createDocument` and `postComment` return `{ id }` and need membership of the context written to;
  `addReaction` returns `{ id }` (idempotent) and `removeReaction` returns `{ ok: true }` (idempotent
  toggle-off). A comment may show a picture its author uploaded to the same context.
- `deleteComment` is its author's or an owner's. A comment that has been answered is emptied and
  stays (`tombstone`), since deleting it would take everyone who answered along; any other goes to
  the context's bin, where `listDeleted` shows it, `restoreComment` brings it back and `purgeComment`
  deletes it for good.
- `setDocumentAuthors` replaces a document's author chips: accounts, or names with no account.
- `moveDocument` takes a document and its subtree to another parent, rewriting every path in it; into
  another context it takes an owner of both, and the threads, files and closed polls go along.
  `copyDocument` makes the same content somewhere else, with files of its own.
- `updateDocument` changes a document (never its slug); `deleteDocument` puts it and its subtree in
  the bin, `restoreDocument` brings back exactly what went together, `purgeDocument` deletes a bin
  entry for good, and `listDeleted` (a query) is the bin of a context. Who may do which is the interim's rule: an author edits a draft, an owner of
  the context edits and arranges anything.
- `inviteMembers` puts people on a roster (one invitation or a whole spreadsheet, skipping whoever is
  already there), `updateMember` and `removeMember` administer it, and a context always keeps an
  owner who can sign in. `listInvitations` and `acceptInvitation` are the caller's own; declining and
  leaving are `removeMember` on one's own row. `parseRoster` reads the office's .xlsx into the rows
  `inviteMembers` takes, so that the spreadsheet stack stays out of the browser.
- A meeting: `createSpeakerList`, `updateSpeakerList`, `deleteSpeakerList`, `clearSpeakerList`,
  `nextSpeaker` and `moveSpeaker` are the chair's; `joinSpeakerList` and `leaveSpeakerList` are any
  member's; `listSpeakerLists` (a query) is what the room follows. `setProjector` and
  `getProjector` are what the projector shows.
- The canvas: `createCanvas` and `setCanvasOpen` are an owner's, `paintCell` is a member's, one
  cell per cooldown, and `getCanvas` (a query) is the board, or with `since` what was painted since.
- Voting: `openPoll` and `closePoll` are the chair's. A poll that is not secret takes
  `castOpenBallot`. A secret one takes two steps that the server cannot join: `issueBallotTokens`
  (signed in: blind signatures on tokens the server cannot read) and `castBallot` (NO session: a
  token and a choice). `getPoll` and `listPolls` (queries) are the state and the tally as the
  caller may see them, `getBoard` is every ballot for whoever may see the counts, and
  `getBoardEntry` is how a voter finds their own, asked as nobody like the cast. `setDelegation`
  gives the caller's vote in a context to another member or takes it back, and `listDelegations`
  (a query) is what stands: a poll freezes it as it opens. Every cast is answered with a receipt and
  every close with a close-out, both signed by the key `getBoardKey` names. `poll`, `ballotEntry` and
  `pollCloseOut` are RECORDS, not methods: what the board account publishes for a poll opened as
  public, and what `wiki-board-mirror` reads.
- `shareToBluesky` posts a page to the caller's own account, on their own PDS.
- `renderMetafile` draws the EMF and WMF figures inside Word and PowerPoint files, which no browser
  can, so that the renderer stays out of the bundle.
- Feedback: `submitFeedback` files a report or a crash, with or without a session; `listFeedback`
  (a query) is all of them for an owner of the site and one's own for anyone else; `deleteFeedback`
  is the site owner's.
- Push: `subscribePush` and `unsubscribePush` are a device's; `notifyContext` is an owner telling a
  context's members something, and `notifyReply` tells an author they were answered. A notification
  only ever links into the app.
- Files: `uploadBlob` takes the raw bytes (not JSON) into a context, `deleteBlob` removes one, and
  `getBlobLink` (a query) signs a short-lived link for an `<iframe>`, a `<video>` or a document
  viewer, none of which can send a header. The bytes themselves are `GET /blob/<id>`, which is plain
  HTTP with ranges and so has no lexicon.
- `claimMembership` binds a pending invitation to the caller by its claim token, and
  `getMemberClaimLink` (a query) gives an active owner the token to hand out. A seat held by an
  account carried over from the interim counts as pending for both: its person takes the whole
  account over by signing in with the address it was registered under, and everyone else is handed
  that one seat by link. On a site that sends mail, `inviteMembers` mails that link to every new seat
  with an address and no account, and `sendInvitation` mails one seat again.

These files are not only documentation. `crates/appview-client` is generated from them, and its
contract tests call every method on the real router with a client that refuses any field a lexicon
does not name, so a lexicon that has drifted from the server fails `cargo test`. After changing one,
run `cargo run -p lexgen` in `crates/`. A field that `null` clears, where leaving it out leaves it
alone, is listed under its object's `nullable`.

Two routes are plain HTTP and have no lexicon: `GET /blob/<id>` (above), and `POST /log`, which takes
the frontend's batched log entries with no session, resolves the wasm frames in their stacks and
forwards them to the log sink with a token the browser never sees.

What is not built yet: `docs/appview-roadmap.md` tracks it.

## NSID

`wiki.radikal.*`: the authority is `radikal.wiki`, the domain the wiki is served on. Taken on 2026-09-20,
when the owner said to do what made most sense of the calls the spaces redesign had put to them; until
then every lexicon sat under the RFC 2606 placeholder `com.example.wiki.*` (the two spikes that hash or
sign over a collection name, `crates/spaces-spike` and `crates/dagcbor-spike`, still do). It rests on the
organization durably holding that domain, registrar and DNS both. A minted record's NSID is effectively
permanent, and so is a space's type in its URI; NOTHING HAS BEEN MINTED, so until the first real record
or space the name is still a find-and-replace away.

Left to do, by whoever holds the DNS: the Lexicon Resolution TXT record on `radikal.wiki`, pointing at
the organization's DID once that account exists (`docs/atproto-spaces-redesign.md`, decision 1).

Names stay flat under the one authority (`wiki.radikal.getNode`, never `wiki.radikal.node.get`): every
further segment is another domain to resolve a lexicon by.

## Codegen pipeline

`Lexicon (these files) -> atrium-lex / atrium-codegen -> Rust record types -> serde (JSON at the XRPC
boundary, deterministic DAG-CBOR/DRISL on-repo)`. The generated types are the source of truth for the
PUBLIC record shape only; the private types are hand-authored Rust, mapped at an explicit publish seam.

## Conventions

- Records are keyed by `tid` (time-sortable rkeys).
- Timestamps are `format: datetime`; references use `com.atproto.repo.strongRef`.
- Numbers are integers only (atproto has no float/decimal); enums are closed `knownValues` strings
  (extend by adding values, never by a breaking change).
- String limits use both `maxLength` (bytes) and `maxGraphemes` so client validation matches enforcement.

## Versioning and evolution

Published records stay readable forever, so a lexicon may only ever grow compatibly:

- New fields are OPTIONAL only. A field added after first publish can never become required,
  because records minted before it exist without it.
- Never retype a field and never promote an optional field to required; both would invalidate
  records already published under the schema.
- Enums (`knownValues` strings) extend only by ADDING values; readers must tolerate values they
  do not know. Removing or renaming a value is a breaking change.
- Any breaking change (a retype, a new required field, a semantic change to an existing field)
  mints a NEW NSID instead of mutating the old one. The old lexicon keeps validating the records
  already published under it, forever.

# Redesign on atproto spaces

atproto gained a way to hold data that is not public: **spaces** (formerly "the
permissioned data protocol"), in alpha since 2026-08-20
([announcement](https://atproto.com/blog/atproto-spaces-alpha),
[proposal 0016](https://github.com/bluesky-social/proposals/tree/main/0016-permissioned-data)).
The owner asked on 2026-09-20 that the wiki use them. This is the redesign: what
the wiki looks like on spaces, what changes in the AppView and what does not,
what it costs, the calls that are the owner's, and the order to build it in.

The premise it replaces is the first line of `atproto-domain-model.md`: "atproto
has no private records. Anything private is simply DB-only." That was true, and
it is why the AppView as built keeps every page, comment and vote in its own
datastore, with atproto for identity alone. It is no longer true.

Spaces are an ALPHA: breaking changes weekly, data on the hosted PDS deleted
without warning, "you absolutely should not run production code against it",
a release "later this year". Nothing here goes near production before that
release. What can be done now is to build against the alpha, which the spike
below did.

## Spaces, as far as this design leans on them

Read from the proposal, and where marked ✓ asked of the real alpha PDS by
`crates/spaces-spike` (`FINDINGS.md` there has the detail).

- A **space** is an access and sync boundary named by an authority DID, a type
  NSID and a key: `at://{authority}/space/{type}/{skey}`. A record is in exactly
  one space: `…/{skey}/{author}/{collection}/{rkey}`. ✓
- Records are NOT stored with the space. Each author keeps their own records
  for a space in a **permissioned repo** on their own PDS. The space is the sum
  of those repos. ✓
- **Reading is all or nothing.** Whoever holds a space credential reads every
  repo in the space. There is no grant by record, collection or author. ✓
- The **authority** decides who gets a credential, and the protocol does not
  say how. Every PDS implements `com.atproto.simplespace`, whose policies are a
  member list, `public`, or `managing-app`: ask an application, per user,
  whether they may read and whether they may write
  (`checkUserAccess`). ✓
- **Nobody is prevented from writing** into their own repo for a space. The
  write policy decides only whose repo is listed in the writer set and whose
  write notifications are forwarded. Applications decide which records count. ✓
- **Sync has no relay and no firehose.** An application asks the space host for
  the writer set (`listRepos`), follows each repo's operation log
  (`listRepoOps`, record values inlined), checks its copy against a signed
  commit over an order-independent set hash, and falls back to the whole repo as
  a CAR (`getRepo`). Write notifications are best-effort hints to pull. ✓
- A commit is **deniable**: the author signs a nonce, and the hash is bound to
  it by a MAC anyone could have made. A reader is sure of what it synced; a
  leaked commit proves nothing to a third party. ✓
- A credential is bound to the application's key (DPoP) and lasts two hours. An
  application gets one through any one user's session, and may be required to
  attest which application it is (`appAccess: #allowList`): a token signed by a
  key its client metadata publishes, which the space's host fetches to check.
  Every application is asked, the authority's own included. ✓
- Users grant access by **space type** through OAuth (`space:<type>?…`), and the
  consent screen shows the type's name.
- Files are blobs on the author's PDS, served through the space. ✓ A PDS as it
  comes refuses blobs over 5 MB. ✓
- Record data has **no fractions**. A PDS refuses a record holding one, or an
  integer past 2^53, or a request past about 1 MB. ✓
- A write into a space that has been **deleted succeeds**, and deleting a space
  or a record twice is no error: whether a space is still there, and still set
  up as it was made, has to be asked (`getSpace`). ✓
- The authority's own account reads its spaces without the managing app being
  asked. ✓
- Spaces give **access control, not confidentiality**. A PDS can read what it
  hosts, and so can every application admitted to a space.
- When an account is deleted, everyone downstream is expected to delete what it
  held of that account, in spaces as in public.

## The shape of it

### One context, one space

The wiki's unit of access is the context: a group, an event, a site, the home.
Membership does not inherit, and a member of an event reaches it through a
group they do not belong to. That is exactly a space, so each context is one:

```text
at://{the organization's DID}/space/{nsid}.context/{context id}
```

The key is the context's id, never its slug: a rename must not move the space.
One space type for every kind of context, since they hold the same kinds of
record and a member consents once ("your wiki groups and events"), with the
kind said by a profile record inside. A nested context is a space of its own,
linked from a record in its parent, so that whoever reads only the child never
needs the parent, as now.

Today's read rule is "every member of the context reads everything in it; a
draft is unlisted, not unreadable" (`crates/appview/src/authz.rs`), which is
what all-or-nothing reading gives. Nothing the wiki shows a member today needs
a narrower grant than a space has.

### One authority: the organization

Every space is anchored on ONE account that is the organization's and nobody's
personally, so that a space outlives whoever started it. The spaces are
`simplespace` spaces on that account's PDS, with both policies set to
`managing-app` and the AppView as the managing app. The PDS then issues
credentials and routes notifications, and asks the AppView who may read and
write, which the AppView answers from the roster it already keeps. No member
list exists anywhere else, which matters because the roster is mostly people who
have no DID yet: seats waiting for an address.

The proposal allows an authority to be served by a host of its own instead of a
PDS. That is more code for more control, and nothing here needs it yet.

### Who holds which record

A record lives in the repo of whoever the wiki's rules say may change it.

| Held by | Records | Why |
|---|---|---|
| The organization, in its own repo in the space | the context's profile (name, kind, parent, front page, cover); folders, documents, files, elections; a poll's announcement, its board and its close-out; everything carried over from the interim | Only owners make these, and they are the group's: an agenda must not vanish because whoever typed it deleted their account. |
| The organization, as an **overlay** on a member's record | a submission (the version frozen, with its text); a placement (moved, renamed or reordered by an owner); a lock; a removal to the bin | An owner cannot edit another account's repo. They say what they did in the organization's, by reference, as the alpha's own sample app does for its removals. |
| The member, in their own repo in the space | a motion, an amendment, a candidacy, a question, a comment, a reaction | Theirs to write, to change until submitted, and to take with them. |
| Nobody: the AppView's own datastore, as now | the roster with addresses, roles, claim links and hidden seats; sessions; eligibility, token issuance, delegation and every ballot; speaker lists, the projector, the canvas; push subscriptions; reports; the map of carried accounts; whether a context is open to everyone, and where a page was published on the open network | Narrower than a space can be (owners only), or has to be atomic and authoritative (a vote), or is gone when the meeting is. |

A **submission** carries the text, not only a reference to it. A member can
always edit or delete what is in their own repo, and deleting their account
obliges everyone to drop it. What a meeting decided on is the meeting's, so the
organization keeps its own copy from the moment of submission, with the author
named as the minutes would name them.

**Held by the organization, for now, for everyone.** A record a member would
hold can equally be held by the organization with the author named inside it.
That form is needed anyway: for everything carried over from the interim, whose
authors have no DID yet, and for any member whose PDS does not speak spaces,
which today is every PDS but the alpha. So the first stage holds EVERYTHING in
the organization's repos, and asks nothing new of any member: no new consent,
no spaces-capable PDS, no dependence on their PDS being up. Moving a kind of
record to its authors' own repos is then a later step, kind by kind, for the
members whose PDS can hold it, under the same lexicons. The owner's call is how
far to take that (see Decisions).

### The AppView's roles

Three, where it has one and a half today.

- **The application members use.** Unchanged in what it serves: the same XRPC
  methods, the same `/ws`, the same sessions. What changes is under a write: it
  authorizes as now, builds a record, writes it to the repo that holds it (the
  organization's through the organization's own session, later a member's
  through theirs), and indexes what it wrote at once, so that a member reads
  their own write without waiting for sync.
- **The managing app.** `checkUserAccess(space, user, access)` from the roster:
  read for a member, or for anyone where the context is open and not in the
  bin, write for a member. Voting rights play no part, as they play none in
  reading today. Asked by the
  organization's PDS under a service token the AppView verifies, and answered
  to nobody else.
- **A syncer.** For every space: register for notifications, pull the log of
  each repo that advanced, verify the commit, and index. Sweep the writer set
  now and then, since notifications are hints. Every record is held to the
  wiki's rules on the way in (who may make what under what, a lock, a closed
  poll), because nobody is stopped from writing: what breaks a rule is not
  indexed. The tables that hold pages, comments and reactions today become this
  index, rebuildable from the repos (`appview reindex`), each row knowing the
  record it came from (`uri`, `cid`, `rev`).

What is proven about sync in Rust so far is the hard part: the set hash and the
commit (`crates/spaces-spike`).

### Voting

The ballot core does not move: eligibility, blind tokens, delegation and the
count stay in the AppView, which has to be the one place a vote is cast.
What moves is the **board**. Today it can be published only for a poll opened
as public, because a public repo tells the world a closed group's counts. In a
space the board of EVERY secret poll is published, to exactly the people who
may read the poll: entries in shuffled batches, the close-out, the custody key,
all in the organization's repo in the context's space. The mirror becomes a
syncer with a credential, run by a member. That a commit is deniable costs
nothing here: what has to be undeniable is signed inside the records, by the
custody key, as now.

The one exception is a poll that hides its tally: its counts and its board are
its context's owners' alone, and a space is never narrower than its members. It
has no board in the space, and keeps the receipts and the close-out the AppView
serves.

### Files

A file is a record of its own in the space (`wiki.radikal.file`), keyed by the
id the wiki already knows it by, with its bytes as a blob on the PDS of whoever
holds the record. Pages, covers and comments go on naming a file by that id, as
they do in the datastore, so one file can be named from many places and none of
them changes when its bytes move. In the first stage the holder is the
organization, where the size limit is the organization's to set (the interim's
largest file is 22.7 MB, a PDS's default limit 5 MB): a file past the limit the
AppView is told of stays with the AppView alone and is counted as such. Only
files that something in the space names are mirrored. A picture sent with a
report is in the same store and is never one of them.

The AppView keeps a copy of every blob it serves, as a syncer may, and goes on
serving `/blob/{id}` behind its own check, since a browser holds no space
credential.

### Signing in

As now. A space that admits applications by a list has each attest which it is,
and what that takes is a key published where the `client_id` points, not a
confidential client: the AppView stays the public client it signs members in
as, adds a `jwks_uri` to its client metadata once spaces are configured, and
attests with a key derived from the secret it already keeps. The confidential
client stays the roadmap's one open box, wanted for long-lived tokens to
members' repos and for nothing here.

One addition comes with the first record that moves to a member's repo: the
OAuth request asks for the wiki's spaces by type (a permission set, shown to the
member under the type's name). In the first stage only the organization's
account grants anything.

### Public contexts

Two of 41 contexts are open to everyone. They are spaces like the rest, with the
managing app answering yes to any reader, and the AppView serves them to the
signed-out as it does today. Opening or closing a context is then an answer that
changes, not data that moves. The record lexicons that already exist for the
open network (a resolution, a statement, a post) stay what they were meant for:
a deliberate act of publishing, apart from this.

### Leaving, deleting, ending

A member who leaves stops getting credentials. What they hold in their own repo
stays theirs, and the AppView drops it from the index if their account goes.
What was submitted stays, in the organization's words. An owner's removal is an
overlay, and the bin lists overlays. Deleting a context deletes the space: the
organization's repo goes, syncers are told to drop their copies, and members'
own records become unreadable to everyone but themselves.

## What does not change

The frontend and the seam it talks through; every XRPC method and `/ws`; the
roster, invitations, claim links and mail; taking an old account over at
sign-in; the ballot core, receipts and close-outs; the deploy unit. The cutover
from the interim does not wait for any of this.

## What it costs

- **An alpha to build on**, with a release date that is somebody else's.
- **A PDS in the path of every write**, where there is a local transaction now.
  The organization's PDS becomes something the wiki cannot run without.
- **Rules enforced twice**: on the AppView's own write path, and again on every
  record that arrives by sync, since a record can be written without the
  AppView.
- **A member's PDS reads what that member wrote** once their records move
  there. Their host already learns that they use the wiki, from the sign-in;
  what is new is the text of their contributions to a political organization's
  internal debate. For a member on a host they did not choose for that, this is
  a reason to leave their records with the organization.
- **Less atomicity.** A page and its authors, a move of a subtree, a purge of a
  bin are one transaction today and several records tomorrow.
- **Backups of two things**: the organization's PDS, and the AppView's private
  tables. Only the index is rebuildable.
- **A record has a largest size**, about 1 MB on the alpha. A page whose body
  would take it past that has the body go as a file of its own, which the
  record names (`contentBlob`), and a reader puts it back. One past what the
  PDS takes as a file too stays in the datastore alone, and is counted.
- **A PDS rate limits an account's writes**, and the first mirror of a wiki
  writes every record once. On the organization's own PDS the limit is the
  organization's to set. Told to slow down, the mirror stops asking and the next
  sweep carries on from what was written.
- **A fraction is not a number** to atproto. Inside a page's body or a node's
  settings one is carried as `wiki.radikal.spaceDefs#number`, in its own digits,
  and read back as the number it was.

## Decisions, as taken

Five calls were the owner's. Put to them with a recommendation each, the answer
on 2026-09-20 was to do what made most sense, so each stands as recommended
(`atproto-open-decisions.md` has them as decided). What is still an ACT of the
owner's is marked.

1. **The organization's account.** One new account that is the organization's
   and nobody's personally anchors every space, on a PDS the organization runs
   beside the AppView: that server holds everything the wiki has and is in the
   path of every write, so its location, backups, uptime and file limit should
   be the organization's. The decision log had the AppView "not run or mandate a
   PDS"; members stay free, the organization's own account does not. THE
   OWNER'S, once spaces are released: make the account (a handle on a domain
   the organization holds, keys with two people and not on the server alone).
2. **The NSID** is `wiki.radikal.*`, after the domain the wiki is served on. It
   names the space TYPE too, which is in every space's URI for good. The
   placeholder was renamed the same day; nothing has been minted, so it stays
   cheap to change until the first real record or space. THE OWNER'S: the DNS
   record for lexicon resolution, and being sure the domain stays theirs.
3. **How far records move to their authors.** Everything with the organization
   first, and a place one may stop: the data is in atproto, other applications
   can be let in to read it, the AppView is rebuildable. Whether members'
   contributions move to members' repos is decided again with the release in
   hand, and with the PDS hosts members actually use.
4. **Which applications may read a space**: the wiki, and tools named to it
   such as the board mirror. An allow list, because this is a political
   organization's internal debate. Built: a deployed AppView makes every space
   with a list of itself and whatever `APPVIEW_SPACES_ALLOWED_CLIENTS` names,
   and the sweep takes back a space someone opened. THE OWNER'S: a `client_id`
   for each tool they let in, which is a client metadata document on a domain
   of theirs with the tool's public key in it.
5. **When**: the cutover from the interim goes ahead on the AppView as built,
   because the interim is the fragile part. Spaces follow their release; that
   second move stays inside the backend and is rehearsed with the same tooling.

Revisited in the decision log by this: "atproto has no private records"
(domain model), PDS-agnostic hosting (true of members, not of the
organization's account), lexicons at the federation boundary only (the boundary
now includes what members see, so content gets lexicons; the private half still
has none), and files (a space's blobs, with the AppView's store as a cache).

## Order of work

None of it in production before spaces are released, and all of it off unless
configured (`APPVIEW_SPACES_*`, `services.wiki-appview.spaces`). Done so far
(2026-09-20), each held to the alpha PDS in a container by `just test-spaces`:

- Of S1: the NSID, the space type and the record lexicons, and
  `crates/wiki-records`, which maps the AppView's rows to records and back and
  is tested to lose nothing either way. What a record does not carry is named
  by a type (`Kept`), so that a rebuild cannot forget it.
- S2 but for the CAR reader, in `crates/atproto-spaces`: the calls, the
  credential flow, a copy of a repo held to its signed commit through an edit, a
  removal and a corruption, write notifications, a file, and the service token a
  PDS calls a managing app under.
- Of S3, in `crates/appview/src/spaces.rs`: `checkUserAccess` answered from the
  roster, `notifyWrite` and `notifySpaceDeleted`, all three to the
  organization's own PDS and nobody else; the AppView's DID document when it is
  named by a `did:web`; and the sweep, which asks of every space whether it is
  still there and still under this AppView, makes again one that is gone, and
  takes back one that someone opened.
- The first half of S4: every page, context, comment and reaction is written to
  both. A context is mirrored as a whole once it has been quiet for two seconds
  (a bin, a move or a purge announces one id and changes a subtree), again on a
  sweep every fifteen minutes, and at start. The mirror compares and does not
  remember, so what failed is done by the next pass.
- Decision 4, the allow list: a deployed AppView's spaces admit it and the
  tools named to it and no other application. It attests with a key derived
  from its secret and published at `/jwks.json`, which the alpha PDS fetched
  and held it to; a member's session through an application the space does not
  name was refused, and through one it names was let in.
- S5: the board of every secret poll whose tally its members may see goes
  into its context's space, as it goes to the board account for a poll held in
  public (`crates/appview/src/board.rs`), and `board-mirror follow --space`
  keeps and recounts it as a member, by an app password of their own account.
- Of S6, files: each file something in a space names is uploaded to the
  organization's PDS once and written as a `wiki.radikal.file`. One nothing
  names any more is let go of. A page too long to be one record sends its body
  the same way.
- Of S6, the gates: `appview mirror-spaces` does one pass and then reads every
  space back through a credential, as any syncer would. It wants the same
  records at the same versions under commits that verify, and it rebuilds every
  row from the records ALONE (a path from the parents' slugs, a context from
  the space a record was found in) to hold it to the row that is there. That is
  the claim this design rests on, asked of real data: the content tables are an
  index the repos can rebuild. It says what differs, by context, record and
  field, and with `--bytes` it fetches every file back and holds it to its hash.
  `scripts/rehearse-spaces.nu` runs all of it on a cutover already rehearsed
  (`just rehearse-spaces <dir>`), against the alpha's PDS in a container that
  is gone when it ends: what the next rehearsal on a copy of production is run
  with, to learn which pages and files are past what a PDS takes.

Not yet: registering for notifications and indexing members' own repos, which
is nothing to do until a member holds a record (S7); the record FIRST, which is
the second half of S4; and the rehearsal on a copy of production, which is
the owner's to start (the rest of S6).

- **S1 Foundations.** The NSID. A space type and record lexicons for content,
  overlays and the board (`content` as the editor's own JSON, tagged with its
  format, until a public representation is wanted). `uri`, `cid`, `rev` and
  holder on every indexed row. The confidential client.
- **S2 Plumbing, in Rust.** A client for `com.atproto.space.*` and
  `com.atproto.simplespace.*`; delegation token to credential with DPoP; the
  commit check and set hash from the spike; a CAR reader. A test that runs the
  alpha PDS in a container, as `test-real-login` runs a PDS today.
- **S3 Managing app and syncer.** `checkUserAccess`, `notifyWrite` and
  `notifySpaceDeleted` under verified service tokens; registration and renewal;
  the sweep; ingest with the rules applied; the index and `/ws`.
- **S4 Writes through records, held by the organization.** Kind by kind,
  written to both until each is proven, then the record first.
- **S5 The board in the space**, and the mirror as a syncer.
- **S6 The move**: the AppView's rows to records and its blobs to the PDS, with
  gates as `appview verify` has them (every indexed row has a record; every
  repo's set hash is the index's), rehearsed on a copy of production.
- **S7 Records held by their authors**, if the owner takes decision 3 that far:
  the permission set and consent, overlays, submission snapshots, and the
  fallback for a member whose PDS cannot hold them.

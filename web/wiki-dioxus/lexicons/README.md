# com.example.wiki.* lexicons (placeholder NSID)

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

## Scope

Only entities that are meaningfully and safely publishable get a lexicon:

- `com.example.wiki.post`: a member's feed post (the social unit).
- `com.example.wiki.statement`: a member's personal public statement.
- `com.example.wiki.resolution`: the org's published outcome of a motion/election.
- `com.example.wiki.comment`: a public comment on a public item.
- `com.example.wiki.reaction`: a member's emoji reaction to a public item
  (comment/post/resolution), addressed by a strongRef. One record per (reactor,
  subject, emoji); deleting the record removes the reaction (toggle). Net-new (the
  old wiki had no reactions), so it maps no legacy mime.
- `com.example.wiki.group` / `com.example.wiki.event`: the opt-in-public container contexts. The
  group/event kind split is carried by the two NSIDs; a record exists only while the context is public.
- `com.example.wiki.poll` / `com.example.wiki.ballotEntry`: the public poll announcement and the
  anonymized bulletin-board entry (repo custody pending an owner call).
- `com.example.wiki.document` is EXCLUDED for now: documents store Slate JSON internally, and the
  public rich-text representation (what a document record's body looks like on the wire) is a
  rewrite-time decision that has not been made yet. No lexicon until it is.

Always-private entities (`voted`, roster/eligibility/delegation, membership-as-affiliation,
projector/speaker) deliberately have NO lexicon. The ballot is SPLIT, not simply private: the
org-side ballot row (eligibility, token issuance, resolved weights) is always-private and has no
lexicon, while the public ANONYMIZED board entry (token + choices, no voter identity) is exactly
what `com.example.wiki.ballotEntry` describes.

## Methods (the AppView's XRPC serving layer)

These describe the AppView's own read/write API (`crates/appview/src/xrpc.rs`), served at
`/xrpc/{nsid}`. They return the AppView's canonical DOMAIN entities (the reconciled internal shapes),
NOT the published repo records above; `com.example.wiki.defs` holds the shared view objects
(`documentView`, `contextView`, `commentView`, `reactionView`, `authorView`) they reference. This is
why a `documentView` exists even though the `document` RECORD is excluded: the served entity shape is
settled, but its public rich-text record shape is not.

Queries (GET, identity-free public reads):

- `getDocument` / `getContext` return a single entity; `resolveNode` walks a slug path to a context.
- `listChildren`, `listContexts`, `listRecent`, `search`, `getComments`, `getReactions` return an
  object wrapping a named array (`{ documents: [...] }`, `{ contexts: [...] }`, ...). Lists are
  wrapped, never a bare top-level array, because a bare array is not a valid lexicon `output.schema`
  and the wrapper leaves room for a future `cursor`.

Procedures (POST, authenticated; the caller's DID comes from the request authorization, not the body):

- `createDocument` and `postComment` return `{ id }`; `addReaction` returns `{ id }` (idempotent) and
  `removeReaction` returns `{ ok: true }` (idempotent toggle-off).

The membership/authz-gated reads and richer write procedures are deferred with the DID-binding flow.

## NSID

`com.example.wiki.*` is a DELIBERATE placeholder: `example.com` is IANA-reserved (RFC 2606), so it can
never collide with a real authority and is universally read as "not yet decided". The owner has not picked
the authority domain yet (tracked as an Open decision in `docs/atproto-open-decisions.md`). The real NSID
authority must be a domain the org durably controls, registered via the Lexicon Resolution mechanism (a
DNS TXT record on that domain pointing at the org DID). Nothing may be published or minted under the
placeholder; a minted record's NSID is effectively permanent.

### Rebranding when the domain is decided

The swap is mechanical. For a decided domain `D` (say `radikal.wiki`, giving authority `wiki.radikal`):

1. `grep -rl 'com\.example\.wiki\.' docs/ lexicons/ | xargs sed -i 's/com\.example\.wiki\./wiki.radikal./g'`
2. `mv lexicons/com/example/wiki lexicons/<reversed-domain-path>` (mirror the NSID segments), then remove
   the empty `com/example` directories.
3. Register the DNS TXT record per Lexicon Resolution and record the decision as Decided in
   `docs/atproto-open-decisions.md`.

Do this BEFORE any codegen output, XRPC route, or Jetstream filter embeds the NSID in code.

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

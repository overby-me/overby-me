# com.example.wiki.* lexicons (placeholder NSID)

atproto Lexicon schemas for the PUBLIC subset of the app's data, drafted for the atproto rewrite
(pre-rewrite plan #7). These define the wire format of records the app publishes to atproto repos so
other AppViews can read them. They are the federation-boundary contract only; the private,
org-authoritative half (ballots, roster, delegation, eligibility, internal deliberation) is owned by
Rust serde types in the backend and never becomes a record. See `docs/atproto-stack-decisions.md`
(Lexicon-to-atrium codegen pipeline) and `docs/atproto-domain-model.md`.

## Scope

Only entities that are meaningfully and safely publishable get a lexicon:

- `com.example.wiki.post`: a member's feed post (the social unit).
- `com.example.wiki.resolution`: the org's published outcome of a motion/election.
- `com.example.wiki.comment`: a public comment on a public item.
- (later) `com.example.wiki.group` / `com.example.wiki.event` / `com.example.wiki.document` for the opt-in-public
  container/content records.

Always-private entities (`ballot`, `voted`, membership-as-affiliation, projector/speaker) deliberately
have NO lexicon.

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

# wiki.radikal.* lexicons

atproto Lexicon schemas for the PUBLIC subset of the app's data, drafted for the atproto rewrite
(pre-rewrite plan #7). These define the wire format of records the app publishes to atproto repos so
other AppViews can read them. They are the federation-boundary contract only; the private,
org-authoritative half (ballots, roster, delegation, eligibility, internal deliberation) is owned by
Rust serde types in the backend and never becomes a record. See `docs/atproto-stack-decisions.md`
(Lexicon-to-atrium codegen pipeline) and `docs/atproto-domain-model.md`.

## Scope

Only entities that are meaningfully and safely publishable get a lexicon:

- `wiki.radikal.post`: a member's feed post (the social unit).
- `wiki.radikal.resolution`: the org's published outcome of a motion/election.
- `wiki.radikal.comment`: a public comment on a public item.
- (later) `wiki.radikal.group` / `wiki.radikal.event` / `wiki.radikal.document` for the opt-in-public
  container/content records.

Always-private entities (`ballot`, `voted`, membership-as-affiliation, projector/speaker) deliberately
have NO lexicon.

## NSID

`wiki.radikal.*` is a PLACEHOLDER. The NSID authority must be a domain the org controls, registered via
the Lexicon Resolution mechanism (a DNS TXT record on that domain pointing at the org DID). Pick the real
domain before publishing any records; a minted record's NSID is effectively permanent.

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
